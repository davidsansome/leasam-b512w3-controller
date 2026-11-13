//! Raw access to the Linux framebuffer (`/dev/fb0`) via mmap. This only writes pixels;
//! making the e-ink panel show them is the refresher's job (see `refresh.rs`).

use super::canvas::{Canvas, Rect};
use super::rotation::Rotation;

#[cfg(target_os = "linux")]
pub use linux::Framebuffer;

#[cfg(not(target_os = "linux"))]
pub use stub::Framebuffer;

#[cfg(target_os = "linux")]
mod linux {
    use std::fs::{File, OpenOptions};
    use std::os::fd::AsRawFd;
    use std::path::Path;

    use anyhow::{Context, Result, bail};
    use log::info;

    use super::{Canvas, Rect, Rotation};

    const FBIOGET_VSCREENINFO: u32 = 0x4600;
    const FBIOGET_FSCREENINFO: u32 = 0x4602;

    #[repr(C)]
    #[derive(Default)]
    struct FbBitfield {
        offset: u32,
        length: u32,
        msb_right: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct FbVarScreeninfo {
        xres: u32,
        yres: u32,
        xres_virtual: u32,
        yres_virtual: u32,
        xoffset: u32,
        yoffset: u32,
        bits_per_pixel: u32,
        grayscale: u32,
        red: FbBitfield,
        green: FbBitfield,
        blue: FbBitfield,
        transp: FbBitfield,
        nonstd: u32,
        activate: u32,
        height: u32,
        width: u32,
        accel_flags: u32,
        pixclock: u32,
        left_margin: u32,
        right_margin: u32,
        upper_margin: u32,
        lower_margin: u32,
        hsync_len: u32,
        vsync_len: u32,
        sync: u32,
        vmode: u32,
        rotate: u32,
        colorspace: u32,
        reserved: [u32; 4],
    }

    #[repr(C)]
    #[derive(Default)]
    struct FbFixScreeninfo {
        id: [u8; 16],
        smem_start: libc::c_ulong,
        smem_len: u32,
        type_: u32,
        type_aux: u32,
        visual: u32,
        xpanstep: u16,
        ypanstep: u16,
        ywrapstep: u16,
        line_length: u32,
        mmio_start: libc::c_ulong,
        mmio_len: u32,
        accel: u32,
        capabilities: u16,
        reserved: [u16; 2],
    }

    pub struct Framebuffer {
        _file: File,
        map: *mut u8,
        map_len: usize,
        width: i32,
        height: i32,
        line_length: usize,
        bytes_per_pixel: usize,
        xoffset: usize,
        yoffset: usize,
        invert: bool,
    }

    impl Framebuffer {
        pub fn open(path: &Path, invert: bool) -> Result<Self> {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .with_context(|| format!("opening {}", path.display()))?;
            let fd = file.as_raw_fd();
            let mut var = FbVarScreeninfo::default();
            let mut fix = FbFixScreeninfo::default();
            // SAFETY: the kernel fills structs of exactly these layouts.
            unsafe {
                if libc::ioctl(fd, FBIOGET_VSCREENINFO as _, &mut var) != 0 {
                    bail!("FBIOGET_VSCREENINFO: {}", std::io::Error::last_os_error());
                }
                if libc::ioctl(fd, FBIOGET_FSCREENINFO as _, &mut fix) != 0 {
                    bail!("FBIOGET_FSCREENINFO: {}", std::io::Error::last_os_error());
                }
            }
            let id = String::from_utf8_lossy(&fix.id).trim_end_matches('\0').to_string();
            info!(
                "framebuffer {id}: {}x{} (virtual {}x{}, offset {},{}), {} bpp, grayscale={}, rotate={}, line_length={}",
                var.xres,
                var.yres,
                var.xres_virtual,
                var.yres_virtual,
                var.xoffset,
                var.yoffset,
                var.bits_per_pixel,
                var.grayscale,
                var.rotate,
                fix.line_length
            );
            if var.bits_per_pixel != 8 && var.bits_per_pixel != 32 {
                bail!("unsupported framebuffer depth {} bpp", var.bits_per_pixel);
            }
            let map_len = fix.smem_len as usize;
            // SAFETY: mapping the device's own memory, length taken from the driver.
            let map = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    map_len,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_SHARED,
                    fd,
                    0,
                )
            };
            if map == libc::MAP_FAILED {
                bail!("mmap framebuffer: {}", std::io::Error::last_os_error());
            }
            Ok(Self {
                _file: file,
                map: map.cast(),
                map_len,
                width: var.xres as i32,
                height: var.yres as i32,
                line_length: fix.line_length as usize,
                bytes_per_pixel: var.bits_per_pixel as usize / 8,
                xoffset: var.xoffset as usize,
                yoffset: var.yoffset as usize,
                invert,
            })
        }

        pub fn size(&self) -> (i32, i32) {
            (self.width, self.height)
        }

        /// Copies the physical region `rect` into the framebuffer, taking each pixel from the
        /// logical `canvas` through `rot`.
        pub fn write_rect(&mut self, canvas: &Canvas, rect: Rect, rot: Rotation) {
            let r = rect.intersect(&Rect::new(0, 0, self.width, self.height));
            let mut line = vec![0u8; r.w.max(0) as usize];
            for y in r.y..r.bottom() {
                if rot.is_identity() {
                    line.copy_from_slice(&canvas.row(y)[r.x as usize..r.right() as usize]);
                } else {
                    for (i, x) in (r.x..r.right()).enumerate() {
                        let (lx, ly) = rot.to_logical(x, y);
                        line[i] = canvas.pixel(lx, ly);
                    }
                }
                let src = &line[..];
                let start = (y as usize + self.yoffset) * self.line_length
                    + (r.x as usize + self.xoffset) * self.bytes_per_pixel;
                let len = src.len() * self.bytes_per_pixel;
                assert!(start + len <= self.map_len);
                // SAFETY: bounds checked against the mapping above.
                let dst = unsafe { std::slice::from_raw_parts_mut(self.map.add(start), len) };
                let invert = self.invert;
                let px = move |v: u8| if invert { 255 - v } else { v };
                if self.bytes_per_pixel == 1 {
                    for (d, &s) in dst.iter_mut().zip(src) {
                        *d = px(s);
                    }
                } else {
                    for (d, &s) in dst.chunks_exact_mut(4).zip(src) {
                        let v = px(s);
                        d.copy_from_slice(&[v, v, v, 0xFF]);
                    }
                }
            }
        }
    }

    impl Drop for Framebuffer {
        fn drop(&mut self) {
            // SAFETY: unmapping exactly what we mapped.
            unsafe {
                libc::munmap(self.map.cast(), self.map_len);
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod stub {
    use std::path::Path;

    use anyhow::{Result, bail};

    use super::{Canvas, Rect, Rotation};

    pub struct Framebuffer;

    impl Framebuffer {
        pub fn open(_path: &Path, _invert: bool) -> Result<Self> {
            bail!("the framebuffer is only available on Linux")
        }

        pub fn size(&self) -> (i32, i32) {
            (0, 0)
        }

        pub fn write_rect(&mut self, _canvas: &Canvas, _rect: Rect, _rot: Rotation) {}
    }
}
