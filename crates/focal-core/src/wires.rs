//! Routes connection wires through the gutters.
//!
//! Rules, in priority order (same as the mock):
//! 1. never under a window — windows plus a clearance are hard obstacles;
//! 2. prefer a private lane — wires may cross at right angles but not
//!    run alongside each other in the same cells (PCB rule);
//! 3. share a lane if the channel is too tight;
//! 4. if no route exists at all, return `path: None` (renderer draws
//!    stubs at the ports).
//!
//! Multiple wires leaving one window edge fan out into distinct ports,
//! ordered by where they're headed.

use std::collections::HashMap;

use crate::geometry::Rect;

/// Routing grid pitch in pixels. At 8K/65" this is about 0.18".
pub const CELL: f32 = 24.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireKind {
    /// Auto-detected relationship (config rule matched).
    Hard,
    /// User-drawn session binding.
    Soft,
}

#[derive(Clone, Copy, Debug)]
pub struct Wire {
    /// Indices into the window-rect slice passed to `route_all`.
    pub a: usize,
    pub b: usize,
    pub kind: WireKind,
}

#[derive(Clone, Debug)]
pub struct Routed {
    pub wire: Wire,
    /// Polyline in pixels, ports included. `None` = unroutable.
    pub path: Option<Vec<(f32, f32)>>,
}

const DX: [i32; 4] = [1, -1, 0, 0];
const DY: [i32; 4] = [0, 0, 1, -1];

fn orient(dir: usize) -> u8 {
    if dir < 2 { 1 } else { 2 }
}

/// Which side of `r` faces `o`: 0=right 1=left 2=bottom 3=top.
fn side_of(r: Rect, o: Rect) -> usize {
    let (rc, oc) = (r.center(), o.center());
    let (dx, dy) = (oc.0 - rc.0, oc.1 - rc.1);
    if dx.abs() > dy.abs() {
        if dx > 0.0 { 0 } else { 1 }
    } else if dy > 0.0 {
        2
    } else {
        3
    }
}

struct Grid {
    gw: i32,
    gh: i32,
    blk: Vec<bool>,
}

impl Grid {
    fn new(w: f32, h: f32, windows: &[Rect], clearance: f32) -> Self {
        let gw = (w / CELL).ceil() as i32;
        let gh = (h / CELL).ceil() as i32;
        let mut blk = vec![false; (gw * gh) as usize];
        for x in 0..gw {
            blk[x as usize] = true;
            blk[((gh - 1) * gw + x) as usize] = true;
        }
        for y in 0..gh {
            blk[(y * gw) as usize] = true;
            blk[(y * gw + gw - 1) as usize] = true;
        }
        for r in windows {
            let e = r.expand(clearance);
            let x0 = ((e.x / CELL).floor() as i32).max(0);
            let x1 = ((e.right() / CELL).floor() as i32).min(gw - 1);
            let y0 = ((e.y / CELL).floor() as i32).max(0);
            let y1 = ((e.bottom() / CELL).floor() as i32).min(gh - 1);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    blk[(y * gw + x) as usize] = true;
                }
            }
        }
        Self { gw, gh, blk }
    }

    fn free(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < self.gw && y < self.gh && !self.blk[(y * self.gw + x) as usize]
    }
}

struct Port {
    border: (f32, f32),
    cell: Option<(i32, i32)>,
}

/// A port on `side` of `r` at fraction `frac` along that side, plus the
/// nearest free routing cell straight out from it.
fn make_port(r: Rect, side: usize, frac: f32, grid: &Grid, clearance: f32) -> Port {
    let (bx, by, px, py) = match side {
        0 => (r.right(), r.y + r.h * frac, 1i32, 0i32),
        1 => (r.x, r.y + r.h * frac, -1, 0),
        2 => (r.x + r.w * frac, r.bottom(), 0, 1),
        _ => (r.x + r.w * frac, r.y, 0, -1),
    };
    for step in 1..14 {
        for off in [0i32, 1, -1, 2, -2, 3, -3] {
            let fx = bx
                + px as f32 * (clearance + CELL * step as f32)
                + if py != 0 { off as f32 * CELL } else { 0.0 };
            let fy = by
                + py as f32 * (clearance + CELL * step as f32)
                + if px != 0 { off as f32 * CELL } else { 0.0 };
            let (gx, gy) = ((fx / CELL).floor() as i32, (fy / CELL).floor() as i32);
            if grid.free(gx, gy) {
                return Port { border: (bx, by), cell: Some((gx, gy)) };
            }
        }
    }
    Port { border: (bx, by), cell: None }
}

/// Breadth-first route preferring straight continuation (fewer bends).
/// `occ` carries lane occupancy: bit 1 horizontal, bit 2 vertical. A
/// move into a cell already used in the same orientation is refused —
/// crossings (perpendicular) pass.
fn bfs(grid: &Grid, occ: Option<&[u8]>, start: (i32, i32), goal: (i32, i32)) -> Option<Vec<(i32, i32)>> {
    let n = (grid.gw * grid.gh) as usize;
    let idx = |x: i32, y: i32| (y * grid.gw + x) as usize;
    if !grid.free(start.0, start.1) || !grid.free(goal.0, goal.1) {
        return None;
    }
    let mut prev: Vec<i32> = vec![-1; n];
    let mut q: Vec<(i32, i32, i8)> = vec![(start.0, start.1, -1)];
    prev[idx(start.0, start.1)] = idx(start.0, start.1) as i32;
    let mut qi = 0usize;
    let mut found = false;
    while qi < q.len() {
        let (x, y, d) = q[qi];
        qi += 1;
        if (x, y) == goal {
            found = true;
            break;
        }
        let order: [usize; 4] = match d {
            0 => [0, 1, 2, 3],
            1 => [1, 0, 2, 3],
            2 => [2, 0, 1, 3],
            3 => [3, 0, 1, 2],
            _ => [0, 1, 2, 3],
        };
        for &nd in &order {
            let (nx, ny) = (x + DX[nd], y + DY[nd]);
            if !grid.free(nx, ny) {
                continue;
            }
            let k = idx(nx, ny);
            if prev[k] >= 0 {
                continue;
            }
            if let Some(o) = occ {
                if o[k] & orient(nd) != 0 {
                    continue;
                }
            }
            prev[k] = idx(x, y) as i32;
            q.push((nx, ny, nd as i8));
        }
    }
    if !found {
        return None;
    }
    let mut pts = Vec::new();
    let mut k = idx(goal.0, goal.1);
    loop {
        pts.push((k as i32 % grid.gw, k as i32 / grid.gw));
        let p = prev[k] as usize;
        if p == k {
            break;
        }
        k = p;
    }
    pts.reverse();
    Some(pts)
}

fn mark(occ: &mut [u8], full: &[(i32, i32)], gw: i32) {
    for i in 1..full.len() {
        let (ax, ay) = full[i - 1];
        let (bx, by) = full[i];
        let o = if ay == by { 1 } else { 2 };
        occ[(ay * gw + ax) as usize] |= o;
        occ[(by * gw + bx) as usize] |= o;
    }
}

fn simplify(pts: &[(i32, i32)]) -> Vec<(i32, i32)> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut simp = vec![pts[0]];
    for i in 1..pts.len() - 1 {
        let a = *simp.last().unwrap();
        let b = pts[i];
        let c = pts[i + 1];
        if (a.0 == b.0 && b.0 == c.0) || (a.1 == b.1 && b.1 == c.1) {
            continue;
        }
        simp.push(b);
    }
    simp.push(*pts.last().unwrap());
    simp
}

/// Route every wire. Order matters (earlier wires get straighter
/// paths), so callers should keep wire order stable across frames.
pub fn route_all(
    screen_w: f32,
    screen_h: f32,
    windows: &[Rect],
    wires: &[Wire],
    clearance: f32,
) -> Vec<Routed> {
    let grid = Grid::new(screen_w, screen_h, windows, clearance);
    let n = (grid.gw * grid.gh) as usize;
    let mut occ = vec![0u8; n];

    // Fan endpoints on the same (window, side) out into distinct ports,
    // ordered by destination so wires don't cross right at the wall.
    let mut groups: HashMap<(usize, usize), Vec<(usize, usize, f32)>> = HashMap::new();
    for (wi, w) in wires.iter().enumerate() {
        let sa = side_of(windows[w.a], windows[w.b]);
        let sb = side_of(windows[w.b], windows[w.a]);
        let sort_a = if sa < 2 { windows[w.b].center().1 } else { windows[w.b].center().0 };
        let sort_b = if sb < 2 { windows[w.a].center().1 } else { windows[w.a].center().0 };
        groups.entry((w.a, sa)).or_default().push((wi, 0, sort_a));
        groups.entry((w.b, sb)).or_default().push((wi, 1, sort_b));
    }
    let mut ports: Vec<[Option<Port>; 2]> = (0..wires.len()).map(|_| [None, None]).collect();
    for ((win, side), mut members) in groups {
        members.sort_by(|p, q| p.2.partial_cmp(&q.2).unwrap());
        let count = members.len();
        for (k, (wi, end, _)) in members.into_iter().enumerate() {
            let frac = (k + 1) as f32 / (count + 1) as f32;
            ports[wi][end] = Some(make_port(windows[win], side, frac, &grid, clearance));
        }
    }

    wires
        .iter()
        .enumerate()
        .map(|(wi, w)| {
            let pa = ports[wi][0].as_ref().unwrap();
            let pb = ports[wi][1].as_ref().unwrap();
            let full = match (pa.cell, pb.cell) {
                (Some(ca), Some(cb)) => {
                    bfs(&grid, Some(&occ), ca, cb).or_else(|| bfs(&grid, None, ca, cb))
                }
                _ => None,
            };
            let path = full.map(|full| {
                mark(&mut occ, &full, grid.gw);
                let simp = simplify(&full);
                let mut v = Vec::with_capacity(simp.len() + 2);
                v.push(pa.border);
                v.extend(
                    simp.iter()
                        .map(|&(x, y)| ((x as f32 + 0.5) * CELL, (y as f32 + 0.5) * CELL)),
                );
                v.push(pb.border);
                v
            });
            Routed { wire: *w, path }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_around_not_under() {
        let windows = vec![
            Rect::new(200.0, 350.0, 400.0, 300.0),
            Rect::new(1400.0, 350.0, 400.0, 300.0),
            // an obstacle square in the middle of the channel
            Rect::new(900.0, 400.0, 200.0, 200.0),
        ];
        let wires = vec![Wire { a: 0, b: 1, kind: WireKind::Hard }];
        let routed = route_all(2000.0, 1000.0, &windows, &wires, 30.0);
        let path = routed[0].path.as_ref().expect("route should exist");
        assert!(path.len() >= 2);
        // No interior point may fall inside any obstacle (ports touch
        // their own window border, so skip first and last).
        for &(x, y) in &path[1..path.len() - 1] {
            for wr in &windows {
                assert!(
                    !wr.expand(5.0).contains(x, y),
                    "wire passes under a window at ({x},{y})"
                );
            }
        }
    }

    #[test]
    fn same_side_wires_get_distinct_ports() {
        let windows = vec![
            Rect::new(100.0, 350.0, 300.0, 300.0),
            Rect::new(1500.0, 100.0, 300.0, 250.0),
            Rect::new(1500.0, 650.0, 300.0, 250.0),
        ];
        // both wires leave window 0's right side
        let wires = vec![
            Wire { a: 0, b: 1, kind: WireKind::Hard },
            Wire { a: 0, b: 2, kind: WireKind::Soft },
        ];
        let routed = route_all(2000.0, 1000.0, &windows, &wires, 30.0);
        let p0 = routed[0].path.as_ref().unwrap()[0];
        let p1 = routed[1].path.as_ref().unwrap()[0];
        assert!(
            (p0.1 - p1.1).abs() > 1.0,
            "ports should fan out, got {p0:?} and {p1:?}"
        );
        // ordered by destination: wire to the top window exits higher
        assert!(p0.1 < p1.1);
    }

    #[test]
    fn boxed_in_window_reports_no_route() {
        // target completely walled off by four obstacles
        let windows = vec![
            Rect::new(100.0, 100.0, 200.0, 150.0),
            Rect::new(900.0, 420.0, 200.0, 160.0),
            Rect::new(700.0, 200.0, 600.0, 150.0), // wall above
            Rect::new(700.0, 650.0, 600.0, 150.0), // wall below
            Rect::new(650.0, 200.0, 100.0, 600.0), // wall left
            Rect::new(1250.0, 200.0, 100.0, 600.0), // wall right
        ];
        let wires = vec![Wire { a: 0, b: 1, kind: WireKind::Hard }];
        let routed = route_all(2000.0, 1000.0, &windows, &wires, 20.0);
        assert!(routed[0].path.is_none(), "boxed-in target must be unroutable");
    }
}
