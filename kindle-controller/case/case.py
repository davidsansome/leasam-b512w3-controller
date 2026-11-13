"""Wall case for the Kindle Paperwhite 4 controller.

Generates printable STLs with manifold3d (pip install manifold3d numpy trimesh):

    python case.py            # writes stl/*.stl

Coordinates (assembled, as mounted): origin at the midpoint between the two
wall-plate screws on the wall surface, x to the right, y up, z out of the wall.
All dimensions in mm.
"""

import math
import os
import sys

import numpy as np
import manifold3d as m3d
from manifold3d import CrossSection, JoinType, Manifold

# --- Kindle (landscape, USB port and power button on the right-hand end) ---
K_W, K_H, K_D = 167.0, 116.0, 8.4
SCREEN_LEFT, SCREEN_RIGHT = 20.0, 23.5   # Kindle edge to active area
SCREEN_TOP, SCREEN_BOTTOM = 13.0, 13.0   # assumed centred vertically
OPEN_MARGIN = 0.75                       # front opening beyond the active area
USB_FROM_TOP = 65.1 + 7.3 / 2           # centre of the micro-USB port (65.1-72.4)
POWER_FROM_TOP = 41.5 + 7.6 / 2          # centre of the power button (41.5-49.1)
POWER_NOTCH_H = 20.0                     # height of the finger notch

# --- Fit ---
CLR = 0.4             # Kindle to pocket walls, each side
Z_CLR = 0.2           # Kindle front to front plate
SKIRT_CLR = 0.15      # front plate skirt to back plate wall

# --- Back plate ---
WALL = 4.0            # side wall thickness (lower part, below the seam)
BACK = 3.0            # panel against the wall
GAP = 6.0             # cable space behind the Kindle
POCKET = 9.0          # space beyond the Kindle's right edge for the flat USB plug (7.5 mm)
LEDGE = 5.0           # width of the ledges the Kindle rests on
STOP_W = 3.0          # right-hand stops that locate the Kindle in the pocket
TOP_STOP = (8.0, 22.0)     # from the Kindle's top edge (above the power button)
BOTTOM_STOP = (5.0, 13.0)  # from the bottom edge (leaves the cable room below the plug)
SCREW_X = 42.0        # 84 mm screw centres
SCREW_D = 4.0         # M3.5 clearance
SCREW_SLOT = 1.0      # horizontal slack each way
OPENING_W, OPENING_H = 64.0, 70.0  # hole to the wall cavity, between the screws
OUTER_R = 5.0         # case corner radius
BASE_CHAMFER = 0.4    # elephant's-foot chamfer on the wall side

# --- Front plate ---
FRONT_T = 2.0
FRONT_FILLET = 1.6
SKIRT_T = 1.6
Z_SEAM = 7.0          # skirt bottom edge (height above the wall)
TAB_W = 18.0
SLIT = 1.0
BUMP = 0.7            # detent bump height (45 degree faces)
GROOVE = 0.9          # matching groove depth in the back plate

SEG = 64

# --- Derived ---
Z_KB = BACK + GAP                 # Kindle back
Z_KF = Z_KB + K_D                 # Kindle front
Z_TOP = Z_KF + Z_CLR              # top of the back plate walls = front plate underside
Z_FRONT = Z_TOP + FRONT_T
INSET = SKIRT_T + SKIRT_CLR       # back plate wall step under the skirt

IN_W = CLR + K_W + CLR + POCKET
IN_H = CLR + K_H + CLR
W = IN_W + 2 * WALL
H = IN_H + 2 * WALL
X0, X1, Y0, Y1 = -W / 2, W / 2, -H / 2, H / 2

KX0 = X0 + WALL + CLR
KX1 = KX0 + K_W
KY0 = -K_H / 2
KY1 = K_H / 2
IX0, IX1 = KX0 - CLR, KX1 + CLR + POCKET   # inner cavity
IY0, IY1 = KY0 - CLR, KY1 + CLR

Z_BUMP = Z_SEAM + 0.2 + 1.2
TABS_X = [0.0, -(W / 2 - 28), W / 2 - 28]
USB_Y = KY1 - USB_FROM_TOP
POWER_Y = KY1 - POWER_FROM_TOP


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


def outline_points(inset, z, r=OUTER_R):
    pts = rrect(X0 + inset, X1 - inset, Y0 + inset, Y1 - inset, r - inset).to_polygons()
    return [(x, y, z) for poly in pts for x, y in poly]


def profiled_slab(z_flat, z_edge, edge_inset_fn, steps=12):
    """Convex slab of the case outline; the edge at z_edge is rounded by
    edge_inset_fn(distance from z_edge) and the far face at z_flat is full size."""
    pts = outline_points(0, z_flat)
    sign = 1 if z_edge > z_flat else -1
    for i in range(steps + 1):
        d = FRONT_FILLET * i / steps
        pts += outline_points(edge_inset_fn(d), z_edge - sign * d)
    return Manifold.hull_points(pts)


def fillet_inset(r):
    """Fillet of radius r, limited to 45 degree overhang near the edge so it
    prints face-down without drooping."""
    steps = 48
    ds = [r * i / steps for i in range(steps + 1)]
    raw = [r - math.sqrt(max(r * r - (r - d) ** 2, 0)) for d in ds]
    lim = raw[:]
    for i in range(steps - 1, -1, -1):
        lim[i] = min(raw[i], lim[i + 1] + (ds[i + 1] - ds[i]))
    return lambda d: float(np.interp(d, ds, lim))


def diamond_bar(x0, x1, y, z, half_diag):
    """Square bar along x, rotated 45 degrees, centred on (y, z)."""
    s = half_diag * math.sqrt(2)
    bar = Manifold.cube((x1 - x0, s, s), center=True).rotate((45, 0, 0))
    return bar.translate(((x0 + x1) / 2, y, z))


# --- Parts ---
def back_plate():
    ch = BASE_CHAMFER
    lower = Manifold.hull_points(outline_points(ch, 0) + outline_points(0, ch) +
                                 outline_points(0, Z_SEAM))
    upper = rbox(X0 + INSET, X1 - INSET, Y0 + INSET, Y1 - INSET, Z_SEAM - 0.01, Z_TOP,
                 OUTER_R - INSET)
    body = lower + upper

    cuts = [rbox(IX0, IX1, IY0, IY1, BACK, Z_TOP + 1, 1.0)]

    # Hole through to the wall, between the screws.
    cuts.append(rbox(-OPENING_W / 2, OPENING_W / 2, -OPENING_H / 2, OPENING_H / 2,
                     -1, BACK + 1, 3))

    # Screw slots.
    for sx in (-SCREW_X, SCREW_X):
        slot = rrect(sx - SCREW_SLOT - SCREW_D / 2, sx + SCREW_SLOT + SCREW_D / 2,
                     -SCREW_D / 2, SCREW_D / 2, SCREW_D / 2)
        cuts.append(Manifold.extrude(slot, BACK + 2).translate((0, 0, -1)))

    # Detent grooves under the front plate's tabs.
    for tx in TABS_X:
        for side in (1, -1):
            y = side * (H / 2 - INSET)
            cuts.append(diamond_bar(tx - TAB_W / 2 - 0.5, tx + TAB_W / 2 + 0.5, y, Z_BUMP,
                                    GROOVE))

    # Finger scoop at the bottom centre, under the skirt edge.
    scoop = Manifold.hull(
        Manifold.cylinder(4, 3, circular_segments=SEG).rotate((-90, 0, 0)).translate((-12, Y0 - 1, Z_SEAM))
        + Manifold.cylinder(4, 3, circular_segments=SEG).rotate((-90, 0, 0)).translate((12, Y0 - 1, Z_SEAM)))
    cuts.append(scoop.translate((0, -(4 - 1 - (INSET - 0.15)), 0)))

    body = body - Manifold.batch_boolean(cuts, m3d.OpType.Add)

    # Ledges the Kindle rests on, leaving the right-hand side open for the cable.
    adds = [
        box(IX0 - 0.01, KX0 + LEDGE, IY0 - 0.01, IY1 + 0.01, BACK - 0.01, Z_KB),
        box(IX0 - 0.01, KX1, KY1 - LEDGE, IY1 + 0.01, BACK - 0.01, Z_KB),
        box(IX0 - 0.01, KX1, IY0 - 0.01, KY0 + LEDGE, BACK - 0.01, Z_KB),
    ]
    for ya, yb in ((KY1 - TOP_STOP[1], KY1 - TOP_STOP[0]),
                   (KY0 + BOTTOM_STOP[0], KY0 + BOTTOM_STOP[1])):
        adds.append(box(KX1 - LEDGE, KX1 + CLR + STOP_W, ya, yb, BACK - 0.01, Z_KB))
        adds.append(box(KX1 + CLR, KX1 + CLR + STOP_W, ya, yb, BACK - 0.01, Z_TOP))
    body = body + Manifold.batch_boolean(adds, m3d.OpType.Add)

    return body - power_notch(KX1 + CLR)


def power_notch(x_from):
    """Finger notch over the power button, open to the front and the side."""
    return rbox(x_from, X1 + 10, POWER_Y - POWER_NOTCH_H / 2, POWER_Y + POWER_NOTCH_H / 2,
                Z_SEAM, Z_FRONT + 1, 3)


def front_plate():
    plate = profiled_slab(Z_TOP, Z_FRONT, fillet_inset(FRONT_FILLET))
    skirt = (rbox(X0, X1, Y0, Y1, Z_SEAM + 0.2, Z_TOP + 0.5, OUTER_R) -
             rbox(X0 + SKIRT_T, X1 - SKIRT_T, Y0 + SKIRT_T, Y1 - SKIRT_T, Z_SEAM, Z_TOP + 1,
                  OUTER_R - SKIRT_T))
    body = plate + skirt

    # Detent bumps on the inside of the tabs.
    bumps = []
    for tx in TABS_X:
        for side in (1, -1):
            y = side * (H / 2 - SKIRT_T)
            bumps.append(diamond_bar(tx - TAB_W / 2, tx + TAB_W / 2, y, Z_BUMP, BUMP))
    body = body + Manifold.batch_boolean(bumps, m3d.OpType.Add)

    cuts = []
    # Slits either side of each tab so it can flex.
    for tx in TABS_X:
        for sx in (tx - TAB_W / 2 - SLIT, tx + TAB_W / 2):
            for side in (1, -1):
                ya, yb = sorted((side * (H / 2 + 1), side * (H / 2 - SKIRT_T - BUMP - 0.5)))
                cuts.append(box(sx, sx + SLIT, ya, yb, Z_SEAM, Z_TOP))

    # Screen opening, chamfered 45 degrees towards the front.
    sx0 = KX0 + SCREEN_LEFT - OPEN_MARGIN
    sx1 = KX1 - SCREEN_RIGHT + OPEN_MARGIN
    sy0 = KY0 + SCREEN_BOTTOM - OPEN_MARGIN
    sy1 = KY1 - SCREEN_TOP + OPEN_MARGIN
    c = FRONT_T
    pts = []
    for z, grow in ((Z_TOP - 0.5, 0), (Z_TOP, 0), (Z_FRONT, c), (Z_FRONT + 0.5, c + 0.5)):
        poly = rrect(sx0 - grow, sx1 + grow, sy0 - grow, sy1 + grow, 1 + grow).to_polygons()
        pts += [(x, y, z) for p in poly for x, y in p]
    cuts.append(Manifold.hull_points(pts))

    cuts.append(power_notch(KX1 + CLR))
    return body - Manifold.batch_boolean(cuts, m3d.OpType.Add)


def kindle():
    return box(KX0, KX1, KY0, KY1, Z_KB, Z_KF)


# --- Print orientation ---
def for_print_back(part):
    return part


def for_print_front(part):
    p = part.rotate((0, 180, 0))
    b = p.bounding_box()
    return p.translate((0, 0, -b[2]))


def test_piece(part, x_from):
    """Right-hand end only: checks the Kindle fit, the plug pocket, the power
    notch and one pair of snap tabs."""
    return part.trim_by_plane((1, 0, 0), x_from)


def write_stl(part, path):
    import trimesh
    mesh = part.to_mesh()
    verts = np.asarray(mesh.vert_properties)[:, :3]
    tris = np.asarray(mesh.tri_verts)
    trimesh.Trimesh(verts, tris, process=False).export(path)
    b = part.bounding_box()
    print(f"{path}: {b[3]-b[0]:.1f} x {b[4]-b[1]:.1f} x {b[5]-b[2]:.1f} mm, "
          f"{part.volume()/1000:.1f} cm3")


def main():
    out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "stl")
    os.makedirs(out, exist_ok=True)
    back, front = back_plate(), front_plate()
    test_x = X1 - 45
    write_stl(for_print_back(back), os.path.join(out, "back_plate.stl"))
    write_stl(for_print_front(front), os.path.join(out, "front_plate.stl"))
    write_stl(for_print_back(test_piece(back, test_x)), os.path.join(out, "test_back_end.stl"))
    write_stl(for_print_front(test_piece(front, test_x)), os.path.join(out, "test_front_end.stl"))
    return back, front


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
