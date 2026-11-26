"""Flat-mount wall case for the Kindle Paperwhite 4 controller.

A variant of case.py: the Kindle lies directly on the back panel, with no
cable space behind it. The case sits off-centre on the wall box so that the
cable can drop from the plug pocket straight into the box through a hole beside
the Kindle's right-hand edge.

Generates printable STLs with manifold3d (pip install manifold3d numpy trimesh):

    python case_flat.py       # writes stl/flat_*.stl

    python case_flat.py narrow  # writes stl/flat_narrow_*.stl

The narrow variant is 10.4 mm narrower: the pocket shrinks, the Kindle moves
right relative to the screws and the cable hole narrows to 10 mm.

Add "dome" (python case_flat.py dome, or narrow dome) for versions that take
the dome-head M3.5 screws supplied with wall plates instead of countersunk
ones: the wall-box screw heads sit in counterbores, so the panel is thicker.
These write stl/flat_dome_*.stl and stl/flat_narrow_dome_*.stl.

Coordinates (assembled, as mounted): origin at the midpoint between the two
wall-box screws on the wall surface, x to the right, y up, z out of the wall.
All dimensions in mm.
"""

import math
import os
import sys

import numpy as np

NARROW = "narrow" in sys.argv[1:]
DOME = "dome" in sys.argv[1:]

import manifold3d as m3d
from manifold3d import CrossSection, JoinType, Manifold

# --- Kindle (landscape, USB port on the right-hand end) ---
K_W, K_H, K_D = 167.0, 116.0, 8.4
SCREEN_LEFT, SCREEN_RIGHT = 20.0, 23.5   # Kindle edge to active area
SCREEN_TOP, SCREEN_BOTTOM = 13.0, 13.0   # assumed centred vertically
OPEN_MARGIN = 0.75                       # front opening beyond the active area

# --- Fit ---
CLR = 0.4             # Kindle to pocket walls, each side
Z_CLR = 0.2           # Kindle front to front plate
SKIRT_CLR = 0.15      # front plate skirt to back plate wall

# --- Back plate ---
WALL = 4.0            # side wall thickness (lower part, below the seam)
BACK = 4.2 if DOME else 3.0  # panel against the wall; the Kindle lies on it
POCKET = 26.6 if NARROW else 37.0  # space beyond the Kindle's right edge for the 90 degree plug
                      # (15.5 mm) and a loop of cable
STOP_W = 3.0          # right-hand stops that locate the Kindle sideways
TOP_STOP = (8.0, 22.0)     # from the Kindle's top edge
BOTTOM_STOP = (5.0, 13.0)  # from the bottom edge
SCREW_X = 42.0        # 84 mm wall-box screw centres
SCREW_TO_EDGE = 13.0  # right-hand screw centre to the case's right edge
SCREW_D = 3.8         # M3.5 clearance
SCREW_CSK = 7.4       # countersink diameter at the panel face (M3.5 head 6.9)
SCREW_SLOT = 1.0      # horizontal slack each way
DOME_HEAD_D = 8.6     # counterbore for dome-head screws (head up to ~8 mm across)
DOME_HEAD_H = 2.7     # counterbore depth (head up to ~2.5 mm high), 0.2 below the face
WALL_SCREW_FROM_LEFT = 20.0  # third screw, into a plasterboard anchor
WALL_SCREW_D = 4.5    # clearance for a 4 mm (#8) countersunk screw
WALL_SCREW_CSK = 9.0  # countersink diameter (head ~8.4)
CAVITY_H = 55.0       # height of the wall box opening; the cable hole spans it
HOLE_TO_SCREW = 8.0   # cable hole's right edge to the right-hand screw centre
PAD_D = 13.0          # locating recesses for stick-on pads (12.7 mm bumpers)
PAD_RECESS = 0.4
OUTER_R = 5.0         # case corner radius
BASE_CHAMFER = 0.4    # elephant's-foot chamfer on the wall side

# --- Front plate ---
FRONT_T = 2.0
FRONT_FILLET = 1.6
SKIRT_T = 1.6
Z_SEAM = 4.0          # skirt bottom edge (height above the wall)
HOOK = 1.0            # top hook rib (half-diagonal of its diamond section)
HOOK_GROOVE = 1.2     # matching groove along the back plate's top wall
HINGE_CHAMFER = 1.5   # ledge chamfer under the top skirt, so the front can tilt
CATCH_W = 18.0        # bottom catches
CATCH_FROM_END = 35.0 # catch centre to the case's left/right edge
SLIT = 1.0
BUMP = 0.5            # catch bump (45 degree faces)
GROOVE = 0.7          # matching groove depth in the back plate

SEG = 64

# --- Derived ---
Z_KB = BACK                       # Kindle back
Z_KF = Z_KB + K_D                 # Kindle front
Z_TOP = Z_KF + Z_CLR              # top of the back plate walls = front plate underside
Z_FRONT = Z_TOP + FRONT_T
INSET = SKIRT_T + SKIRT_CLR       # back plate wall step under the skirt

IN_W = CLR + K_W + CLR + POCKET
IN_H = CLR + K_H + CLR
W = IN_W + 2 * WALL
H = IN_H + 2 * WALL
X1 = SCREW_X + SCREW_TO_EDGE
X0 = X1 - W
Y0, Y1 = -H / 2, H / 2
XC = (X0 + X1) / 2

KX0 = X0 + WALL + CLR
KX1 = KX0 + K_W
KY0 = -K_H / 2
KY1 = K_H / 2
IX0, IX1 = KX0 - CLR, KX1 + CLR + POCKET   # inner cavity
IY0, IY1 = KY0 - CLR, KY1 + CLR

Z_CLIP = Z_SEAM + 0.2 + 1.2       # height of the hook rib and catch bumps
CATCHES_X = [X0 + CATCH_FROM_END, X1 - CATCH_FROM_END]
HOOK_X = (X0 + OUTER_R + 3, X1 - OUTER_R - 3)
WALL_SCREW_X = X0 + WALL_SCREW_FROM_LEFT
HOLE = (KX1, SCREW_X - HOLE_TO_SCREW, -CAVITY_H / 2, CAVITY_H / 2)
PADS = [(X0 + 12, Y0 + 12), (X0 + 12, Y1 - 12), (X1 - 12, Y0 + 12), (X1 - 12, Y1 - 12),
        (-75, Y0 + 12), (-75, Y1 - 12),
        (WALL_SCREW_X, -14), (WALL_SCREW_X, 14)]


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


def countersunk(x0, x1, y, d, csk):
    """Through hole (slotted from x0 to x1) with a 90 degree countersink on the
    Kindle side of the panel, sunk 0.2 mm below the panel face."""
    hole = Manifold.extrude(rrect(x0 - d / 2, x1 + d / 2, y - d / 2, y + d / 2, d / 2),
                            BACK + 2).translate((0, 0, -1))
    depth = (csk - d) / 2
    z_top = BACK + 1
    cone = []
    for x in (x0, x1):
        cone.append(Manifold.cylinder(z_top - (BACK - 0.2 - depth), d / 2,
                                      csk / 2 + (z_top - BACK + 0.2), circular_segments=SEG)
                    .translate((x, y, BACK - 0.2 - depth)))
    return hole + Manifold.hull(cone[0] + cone[1]) if x1 > x0 else hole + cone[0]


def counterbored(x0, x1, y, d, head_d, head_h):
    """Through hole (slotted from x0 to x1) with a flat-bottomed counterbore on
    the Kindle side of the panel, so a dome head sits below the panel face."""
    hole = Manifold.extrude(rrect(x0 - d / 2, x1 + d / 2, y - d / 2, y + d / 2, d / 2),
                            BACK + 2).translate((0, 0, -1))
    bore = Manifold.extrude(rrect(x0 - head_d / 2, x1 + head_d / 2, y - head_d / 2,
                                  y + head_d / 2, head_d / 2), head_h + 1)
    return hole + bore.translate((0, 0, BACK - head_h))


# --- Parts ---
def back_plate():
    ch = BASE_CHAMFER
    lower = Manifold.hull_points(outline_points(ch, 0) + outline_points(0, ch) +
                                 outline_points(0, Z_SEAM))
    upper = rbox(X0 + INSET, X1 - INSET, Y0 + INSET, Y1 - INSET, Z_SEAM - 0.01, Z_TOP,
                 OUTER_R - INSET)
    body = lower + upper

    cuts = [rbox(IX0, IX1, IY0, IY1, BACK, Z_TOP + 1, 1.0)]

    # Cable hole into the wall box, from the Kindle's edge towards the right screw.
    hx0, hx1, hy0, hy1 = HOLE
    cuts.append(rbox(hx0, hx1, hy0, hy1, -1, BACK + 1, 3))

    # Wall-box screws (slotted) and the third screw into a plasterboard anchor.
    for sx in (-SCREW_X, SCREW_X):
        if DOME:
            cuts.append(counterbored(sx - SCREW_SLOT, sx + SCREW_SLOT, 0, SCREW_D,
                                     DOME_HEAD_D, DOME_HEAD_H))
        else:
            cuts.append(countersunk(sx - SCREW_SLOT, sx + SCREW_SLOT, 0, SCREW_D, SCREW_CSK))
    cuts.append(countersunk(WALL_SCREW_X, WALL_SCREW_X, 0, WALL_SCREW_D, WALL_SCREW_CSK))

    # Locating recesses for stick-on pads on the wall side.
    for px, py in PADS:
        cuts.append(Manifold.cylinder(PAD_RECESS + 1, PAD_D / 2, circular_segments=SEG)
                    .translate((px, py, -1)))

    # Hook groove along the top wall; catch grooves on the bottom wall.
    y = H / 2 - INSET
    cuts.append(diamond_bar(HOOK_X[0] - 0.5, HOOK_X[1] + 0.5, y, Z_CLIP, HOOK_GROOVE))
    # Chamfer the ledge under the top skirt so the front plate can tilt about
    # the hook while it is fitted.
    cuts.append(diamond_bar(X0 + OUTER_R, X1 - OUTER_R, Y1, Z_SEAM, HINGE_CHAMFER))
    for cx in CATCHES_X:
        cuts.append(diamond_bar(cx - CATCH_W / 2 - 0.5, cx + CATCH_W / 2 + 0.5, -y, Z_CLIP,
                                GROOVE))

    # Finger scoop at the bottom centre, under the skirt edge.
    scoop = Manifold.hull(
        Manifold.cylinder(4, 3, circular_segments=SEG).rotate((-90, 0, 0)).translate((XC - 12, Y0 - 1, Z_SEAM))
        + Manifold.cylinder(4, 3, circular_segments=SEG).rotate((-90, 0, 0)).translate((XC + 12, Y0 - 1, Z_SEAM)))
    cuts.append(scoop.translate((0, -(4 - 1 - (INSET - 0.15)), 0)))

    body = body - Manifold.batch_boolean(cuts, m3d.OpType.Add)

    # Stops beyond the Kindle's right edge, clear of the cable hole.
    adds = []
    for ya, yb in ((KY1 - TOP_STOP[1], KY1 - TOP_STOP[0]),
                   (KY0 + BOTTOM_STOP[0], KY0 + BOTTOM_STOP[1])):
        adds.append(box(KX1 + CLR, KX1 + CLR + STOP_W, ya, yb, BACK - 0.01, Z_TOP))
    return body + Manifold.batch_boolean(adds, m3d.OpType.Add)


def front_plate():
    plate = profiled_slab(Z_TOP, Z_FRONT, fillet_inset(FRONT_FILLET))
    skirt = (rbox(X0, X1, Y0, Y1, Z_SEAM + 0.2, Z_TOP + 0.5, OUTER_R) -
             rbox(X0 + SKIRT_T, X1 - SKIRT_T, Y0 + SKIRT_T, Y1 - SKIRT_T, Z_SEAM, Z_TOP + 1,
                  OUTER_R - SKIRT_T))
    body = plate + skirt

    # Hook rib along the top, catch bumps on the bottom tabs.
    y = H / 2 - SKIRT_T
    adds = [diamond_bar(HOOK_X[0], HOOK_X[1], y, Z_CLIP, HOOK)]
    for cx in CATCHES_X:
        adds.append(diamond_bar(cx - CATCH_W / 2, cx + CATCH_W / 2, -y, Z_CLIP, BUMP))
    body = body + Manifold.batch_boolean(adds, m3d.OpType.Add)

    cuts = []
    # Slits either side of each catch so it can flex.
    for cx in CATCHES_X:
        for sx in (cx - CATCH_W / 2 - SLIT, cx + CATCH_W / 2):
            cuts.append(box(sx, sx + SLIT, -(H / 2 + 1), -(H / 2 - SKIRT_T - BUMP - 0.5),
                            Z_SEAM, Z_TOP))

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
    """Right-hand end only: checks the Kindle fit, the pocket and cable hole,
    the right-hand screw, a length of the top hook and one bottom catch."""
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


PREFIX = "flat_" + ("narrow_" if NARROW else "") + ("dome_" if DOME else "")


def main():
    out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "stl")
    os.makedirs(out, exist_ok=True)
    back, front = back_plate(), front_plate()
    test_x = X1 - 55
    write_stl(for_print_back(back), os.path.join(out, PREFIX + "back_plate.stl"))
    write_stl(for_print_front(front), os.path.join(out, PREFIX + "front_plate.stl"))
    write_stl(for_print_back(test_piece(back, test_x)),
              os.path.join(out, PREFIX + "test_back_end.stl"))
    write_stl(for_print_front(test_piece(front, test_x)),
              os.path.join(out, PREFIX + "test_front_end.stl"))
    return back, front


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
