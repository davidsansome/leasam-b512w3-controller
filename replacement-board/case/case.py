"""Case for the ESP32 aircon controller board.

Generates printable STLs with manifold3d (pip install manifold3d numpy trimesh matplotlib svg.path):

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
from manifold3d import CrossSection, FillRule, JoinType, Manifold

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
TOP_CLR = 2.0            # above J1/J2 (also covers bridge sag in the low roof)
CAP_CLR = 1.5            # above C2/C3
SIDE_CLR = 0.8           # around C2/C3
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
SKIRT_T = 1.6            # also the lid's wall and roof thickness, except:
ROOF_T = 2.0
TAB_W = 14.0
TAB_X = (18.0, 68.0)
SLIT = 1.0
SLIT_TOP = 5.0           # slits run this far above the tray's top, so the tabs flex
BUMP = 0.7
GROOVE = 0.9
PILLARS = ((8.0, 30.0), (44.0, 53.0), (61.0, 18.0), (62.5, 40.0), (75.0, 21.0))
PILLAR = 3.5             # square, pressing on bare board
FLARE = 4.0              # 45 degree flare where each pillar meets the roof
INNER_T = 1.6            # wall round C2/C3, also standing on the board
LABEL_H = 4.4            # text size (cap height ~3.2)
LABEL_T = 0.6            # raised by (labels and the house)
HOUSE_AT = (63.0, 40.0)  # centre of the waking house on the lid
HOUSE_SCALE = 0.6        # mm per unit of its 68 x 72 drawing grid

SEG = 64

# --- Derived ---
Z_PCB_BOT = FLOOR + SOLDER + UNDER
Z_PCB = Z_PCB_BOT + PCB_T                 # board's top face
Z_SEAM = Z_PCB + 0.05                     # skirt bottom, pillar feet = the lid's print bed
Z_SHOULDER = Z_SEAM - 0.25                # tray step under the skirt
Z_NOTCH = Z_PCB + WIRE_GAP[0]
Z_TT = Z_PCB + WIRE_GAP[1]                # tray top
Z_LOW_IN = Z_PCB + TERM_H + TOP_CLR
Z_LOW = Z_LOW_IN + ROOF_T
Z_HUMP_IN = Z_PCB + CAP_H + CAP_CLR
Z_HUMP = Z_HUMP_IN + ROOF_T

INSET = SKIRT_T + SKIRT_CLR
IX0, IX1 = -CLR, PCB_W + ANTENNA + ANT_CLR      # tray cavity
IY0, IY1 = -CLR, PCB_H + CLR
X0, X1 = IX0 - WALL, IX1 + WALL
Y0, Y1 = IY0 - WALL, IY1 + WALL
W, H = X1 - X0, Y1 - Y0

# The raised part of the lid over C2/C3, and the wall round them that stands on the board.
HX_IN = max(cx for cx, _ in CAPS) + CAP_R + SIDE_CLR
HY_IN = max(cy for _, cy in CAPS) + CAP_R + SIDE_CLR
HX1, HY1 = HX_IN + INNER_T, HY_IN + INNER_T

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


def text(s, x, y, size, align="left"):
    """Text outline as a CrossSection, its baseline-left (or -right) at (x, y)."""
    from matplotlib.font_manager import FontProperties
    from matplotlib.textpath import TextPath
    tp = TextPath((0, 0), s, size=size, prop=FontProperties(family="DejaVu Sans", weight="bold"))
    polys = [p for p in tp.to_polygons(closed_only=True) if len(p) > 2]
    cs = CrossSection([[(float(a), float(b)) for a, b in p] for p in polys], FillRule.EvenOdd)
    b = cs.bounds()
    dx = x - (b[2] if align == "right" else b[0])
    return cs.translate((dx, y))


def stroke(paths, dots, scale, width, centre):
    """SVG line art stroked with round caps and joins, as a CrossSection. The drawing's
    y runs down, so it is flipped; (32, 30) on its grid lands on `centre`."""
    from svg.path import parse_path, Move
    cx, cy = centre

    def pt(z):
        return (cx + (z.real - 32) * scale, cy - (z.imag - 30) * scale)

    r = width / 2
    parts = []
    for d in paths:
        for seg in parse_path(d):
            if isinstance(seg, Move):
                continue
            n = max(2, int(seg.length() * scale / 0.3) + 1)
            pts = [pt(seg.point(i / (n - 1))) for i in range(n)]
            for a, b in zip(pts, pts[1:]):
                parts.append(CrossSection.hull(
                    CrossSection.circle(r, 24).translate(a) + CrossSection.circle(r, 24).translate(b)))
    for x, y, rr in dots:
        parts.append(CrossSection.circle(rr * scale + r, 32).translate(pt(complex(x, y))))
    return CrossSection.batch_boolean(parts, m3d.OpType.Add)


def waking_house(centre, scale):
    """The waking house from the Kindle's waking screen (kindle-controller/src/display/
    icons.rs, draw_waking_house): open eyes, a smile, stretching arms and Wi-Fi arcs."""
    paths = [
        "M14 36 L32 21 L50 36M18 33 V55 H46 V33",
        "M28.5 47.5 q3.5 3 7 0",
        "M18 44 L10 37 l-2 -4M10 37 l-4 -1",
        "M46 44 L54 37 l2 -4M54 37 l4 -1",
        "M26 11 a8.5 8.5 0 0 1 12 0",
        "M21.5 6.5 a15 15 0 0 1 21 0",
    ]
    dots = [(25.5, 40.5, 1.4), (38.5, 40.5, 1.4), (32.0, 15.5, 0.6)]
    return stroke(paths, dots, scale, 2.6 * scale, centre)


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
    # Posts under the points the lid presses on, so the board can't bow.
    for px, py in PILLARS:
        adds.append(box(px - PILLAR / 2, px + PILLAR / 2, py - PILLAR / 2, py + PILLAR / 2,
                        FLOOR - 0.01, zl))
    mid_y = (HY_IN + HY1) / 2
    mid_x = (HX_IN + HX1) / 2
    for px, py in ((12.0, mid_y), (25.0, mid_y), (mid_x, 6.0)):
        adds.append(box(px - 1.75, px + 1.75, py - 1.75, py + 1.75, FLOOR - 0.01, zl))

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
    rh = OUTER_R
    shell = rbox(X0, X1, Y0, Y1, Z_SEAM, Z_LOW, rh)
    hump = Manifold.extrude(
        (rrect(X0, HX1 + rh, Y0, HY1 + rh, rh) ^ CrossSection.square((HX1 - X0, HY1 - Y0)).translate((X0, Y0))),
        Z_HUMP - Z_SEAM).translate((0, 0, Z_SEAM))
    body = shell + hump

    # Inside: everything, less the wall round C2/C3, which stands on the board.
    inner = rbox(X0 + SKIRT_T, X1 - SKIRT_T, Y0 + SKIRT_T, Y1 - SKIRT_T, Z_SEAM - 1, Z_LOW_IN,
                 OUTER_R - SKIRT_T)
    inner = inner - box(X0 - 1, HX1, Y0 - 1, HY1, Z_SEAM - 2, Z_LOW + 1)
    hump_in = box(X0 + SKIRT_T, HX_IN, Y0 + SKIRT_T, HY_IN, Z_SEAM - 1, Z_HUMP_IN)
    # Room for the tray's upper wall.
    ring = (rbox(X0 + SKIRT_T, X1 - SKIRT_T, Y0 + SKIRT_T, Y1 - SKIRT_T, Z_SEAM - 1, Z_TT + 0.1,
                 OUTER_R - SKIRT_T)
            - box(IX0 + 0.2, IX1 - 0.2, IY0 + 0.2, IY1 - 0.2, Z_SEAM - 2, Z_TT + 2))
    body = body - inner - hump_in - ring

    adds = []
    # End walls: solid over the tray's wall, so they close the top of each notch.
    for xa, xb in ((X0 + 0.5, IX0), (IX1, X1 - 0.5)):
        adds.append(box(xa, xb, Y0 + 1, Y1 - 1, Z_TT + 0.1, Z_LOW_IN + 0.01))

    # Pillars pressing on bare board, flaring into the roof to shorten its bridges.
    for px, py in PILLARS:
        p = PILLAR / 2
        adds.append(box(px - p, px + p, py - p, py + p, Z_SEAM, Z_LOW_IN + 0.01))
        f = p + FLARE
        pts = [(px + sx * p, py + sy * p, Z_LOW_IN - FLARE) for sx in (-1, 1) for sy in (-1, 1)]
        pts += [(px + sx * f, py + sy * f, Z_LOW_IN + 0.01) for sx in (-1, 1) for sy in (-1, 1)]
        adds.append(Manifold.hull_points(pts))

    # Bumps on the tabs.
    for tx in TAB_X:
        for side in (1, -1):
            y = Y1 - SKIRT_T if side > 0 else Y0 + SKIRT_T
            adds.append(diamond_bar(tx - TAB_W / 2, tx + TAB_W / 2, y, Z_BUMP, BUMP))

    # Labels over each comb.
    labels = [("POWER", J1_PINS[0]), ("COMMON", J1_PINS[1]), ("AUX", J1_PINS[2])]
    sz = LABEL_H
    for s, y in labels:
        cs = text(s, 1.0, y - sz * 0.36, sz)
        adds.append(Manifold.extrude(cs, LABEL_T + 0.01).translate((0, 0, Z_LOW - 0.01)))
    for s, y in (("TABLET +", J2_PINS[0]), ("TABLET −", J2_PINS[1])):
        cs = text(s, IX1 - 1.0, y - sz * 0.36, sz, align="right")
        adds.append(Manifold.extrude(cs, LABEL_T + 0.01).translate((0, 0, Z_LOW - 0.01)))

    house = waking_house(HOUSE_AT, HOUSE_SCALE)
    adds.append(Manifold.extrude(house, LABEL_T + 0.01).translate((0, 0, Z_LOW - 0.01)))

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
    # Upright: skirt, pillars and the wall round C2/C3 all start on the bed.
    return part.translate((0, 0, -Z_SEAM))


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
