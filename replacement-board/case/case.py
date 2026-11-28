"""Case for the ESP32 aircon controller board.

Generates printable STLs with manifold3d (pip install manifold3d numpy trimesh):

    python case.py            # writes stl/*.stl

Coordinates (assembled): x right and y up as the board is drawn in KiCad, with the
origin at the board's bottom-left corner (KiCad (80, 110)). z is up from the
underside of the tray. All dimensions in mm.
"""

import math
import os
import sys

import numpy as np
import manifold3d as m3d
from manifold3d import CrossSection, JoinType, Manifold

# --- Board (from aircon-esp32-board.kicad_pcb) ---
PCB_W, PCB_H, PCB_T = 80.5, 60.0, 1.4
SOLDER = 2.5             # through-hole solder below the board
ANTENNA = 6.0            # ESP32 antenna overhang past the right-hand edge
CAP_H = 30.0             # C2/C3 above the board
TERM_H = 15.0            # J1/J2 above the board
CAPS = ((8.99, 10.25), (27.24, 10.25))   # C2, C3 centres
CAP_R = 8.0
J1_PINS = (55.09, 50.01, 44.92)          # POWER, COMMON, AUX (left-hand edge)
J2_PINS = (4.20, 9.28)                   # TAB_OUT, GND (right-hand edge)
WIRE_GAP = (2.6, 6.1)    # wire entry, above the board's top face

# --- Fit ---
CLR = 0.3                # board to tray walls
ANT_CLR = 0.5            # beyond the antenna tip
UNDER = 0.5              # below the solder
CAP_CLR = 1.5            # above C2/C3, the tallest parts
SKIRT_CLR = 0.15         # lid skirt to tray wall

# --- Tray ---
FLOOR = 2.0
WALL = 3.8               # below the seam; the part under the skirt is WALL - INSET
LEDGE = 1.5              # strip the board rests on, round its edge
STOP_W = 2.0             # right-hand stop (the antenna side has a gap)
OUTER_R = 3.0
BASE_CHAMFER = 0.4
NOTCH_W = 2.2            # one notch per wire
ZIP = True               # cable-tie anchors outside each comb
ZIP_SLOT = (4.5, 2.0)    # tunnel for the tie (width, height)
ZIP_LUG = (7.0, 12.0, 4.6)   # how far it sticks out, its width, its height

# --- Lid ---
SKIRT_T = 1.6            # also the lid's wall thickness
ROOF_T = 2.0
TAB_W = 14.0
TAB_X = (18.0, 68.0)
SLIT = 1.0
SLIT_TOP = 5.0           # slits run this far above the tray's top, so the tabs flex
BUMP = 0.7
GROOVE = 0.9
# Strips (x0, x1, y0, y1) hanging from the roof that press on bare board, clear of every
# part's courtyard by 0.6 mm and of the wires' path to J1/J2. Each has a post under it.
PRESS = ((0.5, 23.0, 26.5, 29.5), (40.0, 59.0, 50.0, 53.0), (61.5, 64.5, 7.25, 55.0),
         (66.0, 79.75, 17.5, 20.5))
POST = 3.5               # square posts in the tray, under the strips
POSTS = ((11.75, 28.0), (49.5, 51.5), (63.0, 15.0), (63.0, 45.0), (73.0, 19.0))

SEG = 64

# --- Derived ---
Z_PCB_BOT = FLOOR + SOLDER + UNDER
Z_PCB = Z_PCB_BOT + PCB_T                 # board's top face
Z_SEAM = Z_PCB + 0.05                     # skirt bottom, feet of the strips
Z_SHOULDER = Z_SEAM - 0.25                # tray step under the skirt
Z_NOTCH = Z_PCB + WIRE_GAP[0]
Z_TT = Z_PCB + WIRE_GAP[1]                # tray top
Z_TOP_IN = Z_PCB + CAP_H + CAP_CLR        # underside of the lid's roof
Z_TOP = Z_TOP_IN + ROOF_T

INSET = SKIRT_T + SKIRT_CLR
IX0, IX1 = -CLR, PCB_W + ANTENNA + ANT_CLR      # tray cavity
IY0, IY1 = -CLR, PCB_H + CLR
X0, X1 = IX0 - WALL, IX1 + WALL
Y0, Y1 = IY0 - WALL, IY1 + WALL
W, H = X1 - X0, Y1 - Y0

Z_BUMP = (Z_SEAM + Z_TT) / 2
SCOOP_X = 43.0


# --- Helpers ---
def box(x0, x1, y0, y1, z0, z1):
    return Manifold.cube((x1 - x0, y1 - y0, z1 - z0)).translate((x0, y0, z0))


def rrect(x0, x1, y0, y1, r):
    w, h = x1 - x0, y1 - y0
    r = max(min(r, w / 2 - 0.01, h / 2 - 0.01), 0.01)
    cs = CrossSection.square((w - 2 * r, h - 2 * r), center=True)
    cs = cs.offset(r, JoinType.Round, circular_segments=SEG)
    return cs.translate(((x0 + x1) / 2, (y0 + y1) / 2))


def rbox(x0, x1, y0, y1, z0, z1, r):
    return Manifold.extrude(rrect(x0, x1, y0, y1, r), z1 - z0).translate((0, 0, z0))


def outline_points(inset, z):
    pts = rrect(X0 + inset, X1 - inset, Y0 + inset, Y1 - inset, OUTER_R - inset).to_polygons()
    return [(x, y, z) for poly in pts for x, y in poly]


def diamond_bar(x0, x1, y, z, half_diag):
    """Square bar along x, rotated 45 degrees, centred on (y, z)."""
    s = half_diag * math.sqrt(2)
    bar = Manifold.cube((x1 - x0, s, s), center=True).rotate((45, 0, 0))
    return bar.translate(((x0 + x1) / 2, y, z))


def notch_ys():
    return [(X0, y) for y in J1_PINS] + [(X1, y) for y in J2_PINS]


# --- Tray ---
def tray():
    ch = BASE_CHAMFER
    lower = Manifold.hull_points(outline_points(ch, 0) + outline_points(0, ch) +
                                 outline_points(0, Z_SHOULDER))
    upper = rbox(X0 + INSET, X1 - INSET, Y0 + INSET, Y1 - INSET, Z_SHOULDER - 0.01, Z_TT,
                 OUTER_R - INSET)
    body = lower + upper

    cuts = [rbox(IX0, IX1, IY0, IY1, FLOOR, Z_TT + 1, 0.5)]

    # Combs: a notch for each wire, from the top of the wall down to the wire gap.
    for xe, y in notch_ys():
        x0, x1 = (X0 - 1, IX0 + 0.01) if xe == X0 else (IX1 - 0.01, X1 + 1)
        cuts.append(box(x0, x1, y - NOTCH_W / 2, y + NOTCH_W / 2, Z_NOTCH, Z_TT + 1))

    # Detent grooves under the lid's tabs.
    for tx in TAB_X:
        for side in (1, -1):
            y = Y1 - INSET if side > 0 else Y0 + INSET
            cuts.append(diamond_bar(tx - TAB_W / 2 - 0.5, tx + TAB_W / 2 + 0.5, y, Z_BUMP, GROOVE))

    # Finger scoop under the skirt on the top long side.
    r = 3.0
    scoop = Manifold.hull(
        Manifold.cylinder(4, r, circular_segments=SEG).rotate((90, 0, 0)).translate((SCOOP_X - 8, Y1 + 1, Z_SHOULDER))
        + Manifold.cylinder(4, r, circular_segments=SEG).rotate((90, 0, 0)).translate((SCOOP_X + 8, Y1 + 1, Z_SHOULDER)))
    cuts.append(scoop.translate((0, r - 1 - (INSET - 0.15), 0)))

    body = body - Manifold.batch_boolean(cuts, m3d.OpType.Add)

    # Ledges round the board's edge, clear of the through-hole solder.
    zl = Z_PCB_BOT
    adds = [
        box(IX0 - 0.01, LEDGE, IY0 - 0.01, IY1 + 0.01, FLOOR - 0.01, zl),
        box(IX0 - 0.01, PCB_W, IY0 - 0.01, LEDGE, FLOOR - 0.01, zl),
        box(IX0 - 0.01, PCB_W, PCB_H - LEDGE, IY1 + 0.01, FLOOR - 0.01, zl),
        box(PCB_W - LEDGE, PCB_W + CLR, IY0 - 0.01, IY1 + 0.01, FLOOR - 0.01, zl),
        # Right-hand stop, kept below the board's top face so the antenna clears it.
        box(PCB_W + CLR, PCB_W + CLR + STOP_W, IY0 - 0.01, IY1 + 0.01, FLOOR - 0.01, Z_PCB_BOT + 1.0),
    ]
    # Posts under the lid's strips, so the board can't bow.
    for px, py in POSTS:
        adds.append(box(px - POST / 2, px + POST / 2, py - POST / 2, py + POST / 2,
                        FLOOR - 0.01, zl))

    # Cable-tie anchors: a lug at floor level with a tunnel for the tie. The cable
    # sheath lies on top of the lug, and the tie goes round it and through the tunnel.
    if ZIP:
        lx, lw, lh = ZIP_LUG
        sw, sh = ZIP_SLOT
        for xe, ys in ((X0, J1_PINS), (X1, J2_PINS)):
            yc = (min(ys) + max(ys)) / 2
            sgn = -1 if xe == X0 else 1
            xa, xb = sorted((xe - sgn * 1.0, xe + sgn * lx))
            lug = rbox(xa, xb, yc - lw / 2, yc + lw / 2, 0, lh, 1.0)
            t0 = xe + sgn * 1.0
            ta, tb = sorted((t0, t0 + sgn * sw))
            lug = lug - box(ta, tb, yc - lw, yc + lw, 0.8, 0.8 + sh)
            adds.append(lug)

    return body + Manifold.batch_boolean(adds, m3d.OpType.Add)


# --- Lid ---
def lid():
    body = rbox(X0, X1, Y0, Y1, Z_SEAM, Z_TOP, OUTER_R)
    inner = rbox(X0 + SKIRT_T, X1 - SKIRT_T, Y0 + SKIRT_T, Y1 - SKIRT_T, Z_SEAM - 1, Z_TOP_IN,
                 OUTER_R - SKIRT_T)
    # Room for the tray's upper wall.
    ring = (rbox(X0 + SKIRT_T, X1 - SKIRT_T, Y0 + SKIRT_T, Y1 - SKIRT_T, Z_SEAM - 1, Z_TT + 0.1,
                 OUTER_R - SKIRT_T)
            - box(IX0 + 0.2, IX1 - 0.2, IY0 + 0.2, IY1 - 0.2, Z_SEAM - 2, Z_TT + 2))
    body = body - inner - ring

    adds = []
    # End walls: solid over the tray's wall, so they close the top of each notch.
    for xa, xb in ((X0 + 0.5, IX0), (IX1, X1 - 0.5)):
        adds.append(box(xa, xb, Y0 + 1, Y1 - 1, Z_TT + 0.1, Z_TOP_IN + 0.01))

    # Strips hanging from the roof that press on bare board.
    for x0, x1, y0, y1 in PRESS:
        adds.append(box(x0, x1, y0, y1, Z_SEAM, Z_TOP_IN + 0.01))

    # Bumps on the tabs.
    for tx in TAB_X:
        for side in (1, -1):
            y = Y1 - SKIRT_T if side > 0 else Y0 + SKIRT_T
            adds.append(diamond_bar(tx - TAB_W / 2, tx + TAB_W / 2, y, Z_BUMP, BUMP))

    body = body + Manifold.batch_boolean(adds, m3d.OpType.Add)

    cuts = []
    # Slits either side of each tab.
    for tx in TAB_X:
        for sx in (tx - TAB_W / 2 - SLIT, tx + TAB_W / 2):
            for side in (1, -1):
                ya, yb = sorted((side * 1e3, (Y1 - SKIRT_T - BUMP - 0.5) if side > 0 else (Y0 + SKIRT_T + BUMP + 0.5)))
                cuts.append(box(sx, sx + SLIT, ya, yb, Z_SEAM - 1, Z_TT + SLIT_TOP))
    # Slots in the skirt in front of each notch, open at the bottom.
    for xe, y in notch_ys():
        x0, x1 = (X0 - 1, X0 + SKIRT_T + 0.01) if xe == X0 else (X1 - SKIRT_T - 0.01, X1 + 1)
        cuts.append(box(x0, x1, y - NOTCH_W / 2, y + NOTCH_W / 2, Z_SEAM - 1, Z_TT + 0.1))

    return body - Manifold.batch_boolean(cuts, m3d.OpType.Add)


# --- Stand-ins for checking fit ---
def board():
    parts = [box(0, PCB_W, 0, PCB_H, Z_PCB_BOT, Z_PCB)]
    for cx, cy in CAPS:
        parts.append(Manifold.cylinder(CAP_H, CAP_R, circular_segments=SEG).translate((cx, cy, Z_PCB)))
    parts.append(box(1.4, 12.54, 41.88, 58.12, Z_PCB, Z_PCB + TERM_H))        # J1
    parts.append(box(67.21, PCB_W - 2.4, 1.16, 12.33, Z_PCB, Z_PCB + TERM_H))  # J2
    parts.append(box(56.73, 59.27, 32.99, 49.23, Z_PCB, Z_PCB + 9.0))          # J3
    parts.append(box(65.95, PCB_W + ANTENNA, 32.1, 50.1, Z_PCB, Z_PCB + 3.2))  # U2 + antenna
    return Manifold.batch_boolean(parts, m3d.OpType.Add)


# --- Print orientation ---
def for_print_tray(part):
    return part


def for_print_lid(part):
    # Upside down, on its flat top: the strips and skirt grow straight up from the roof.
    return part.rotate((180, 0, 0)).translate((0, 0, Z_TOP))


def test_piece(part, x_from):
    """Right-hand end: the J2 comb, the antenna clearance, one pair of tabs."""
    return part.trim_by_plane((1, 0, 0), x_from)


def write_stl(part, path):
    import trimesh
    mesh = part.to_mesh()
    verts = np.asarray(mesh.vert_properties)[:, :3]
    tris = np.asarray(mesh.tri_verts)
    trimesh.Trimesh(verts, tris, process=False).export(path)
    b = part.bounding_box()
    print(f"{os.path.basename(path)}: {b[3]-b[0]:.1f} x {b[4]-b[1]:.1f} x {b[5]-b[2]:.1f} mm, "
          f"{part.volume()/1000:.1f} cm3")


def main():
    out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "stl")
    os.makedirs(out, exist_ok=True)
    t, l = tray(), lid()
    test_x = TAB_X[1] - TAB_W / 2 - 4
    write_stl(for_print_tray(t), os.path.join(out, "tray.stl"))
    write_stl(for_print_lid(l), os.path.join(out, "lid.stl"))
    write_stl(for_print_tray(test_piece(t, test_x)), os.path.join(out, "test_tray_end.stl"))
    write_stl(for_print_lid(test_piece(l, test_x)), os.path.join(out, "test_lid_end.stl"))
    return t, l


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
