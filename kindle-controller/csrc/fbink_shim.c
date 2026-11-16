// A thin layer over libfbink, so the Rust side doesn't have to mirror FBInkConfig (a big
// struct that changes between FBInk releases). Everything here is plain ints and strings.

#include <errno.h>
#include <stdbool.h>
#include <stdint.h>
#include <strings.h>

#include "fbink.h"

// Waveform names as the fbink CLI's -W option spells them.
static const struct {
	const char*      name;
	WFM_MODE_INDEX_T mode;
} waveforms[] = {
	{ "AUTO", WFM_AUTO },
	{ "DU", WFM_DU },
	{ "GC16", WFM_GC16 },
	{ "GC4", WFM_GC4 },
	{ "A2", WFM_A2 },
	{ "GL16", WFM_GL16 },
	{ "REAGL", WFM_REAGL },
	{ "REAGLD", WFM_REAGLD },
	{ "GC16_FAST", WFM_GC16_FAST },
	{ "GL16_FAST", WFM_GL16_FAST },
	{ "DU4", WFM_DU4 },
	{ "GL4", WFM_GL4 },
	{ "GL16_INV", WFM_GL16_INV },
	{ "GCK16", WFM_GCK16 },
	{ "GLKW16", WFM_GLKW16 },
	{ "INIT", WFM_INIT },
	{ "GC16HQ", WFM_GC16HQ },
};

// The waveform mode for a name (case-insensitive), or -1 if FBInk doesn't know it.
int aircon_fbink_waveform(const char* name)
{
	for (unsigned i = 0; i < sizeof(waveforms) / sizeof(waveforms[0]); i++) {
		if (strcasecmp(name, waveforms[i].name) == 0) {
			return waveforms[i].mode;
		}
	}
	return -1;
}

const char* aircon_fbink_version(void)
{
	return fbink_version();
}

// Opens the framebuffer and lets FBInk identify the device. Returns the fd, or -errno.
int aircon_fbink_open(void)
{
	int fd = fbink_open();
	if (fd < 0) {
		return -(errno ? errno : EIO);
	}
	FBInkConfig cfg = { 0 };
	cfg.is_quiet    = true;
	if (fbink_init(fd, &cfg) != 0) {
		int err = errno ? errno : EIO;
		fbink_close(fd);
		return -err;
	}
	return fd;
}

// Refreshes a region, as `fbink -s top=..,left=..,width=..,height=.. -W mode [-f]` would.
// Returns 0, or -errno.
int aircon_fbink_refresh(int fd, uint32_t top, uint32_t left, uint32_t width, uint32_t height, int mode, bool flashing)
{
	FBInkConfig cfg = { 0 };
	cfg.is_quiet    = true;
	cfg.wfm_mode    = (WFM_MODE_INDEX_T) mode;
	cfg.is_flashing = flashing;
	errno           = 0;
	if (fbink_refresh(fd, top, left, width, height, &cfg) != 0) {
		return -(errno ? errno : EIO);
	}
	return 0;
}

void aircon_fbink_close(int fd)
{
	fbink_close(fd);
}
