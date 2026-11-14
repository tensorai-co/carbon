//! Outlines filled as coverage: each line adds the signed area it covers to a grid of cells, and a running sum along
//! each row gives every pixel's coverage, exactly, for antialiased glyphs. Curves are cut into lines first. Contours
//! wound the same way and overlapping (as variable fonts' are) cover their overlap once; those wound against each
//! other leave holes, as the nonzero rule does for the outlines fonts have.

/// A step of an outline in pixels, y down.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    Move(f32, f32),
    Line(f32, f32),
    Quad(f32, f32, f32, f32),
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
}

/// The smallest pixel box around an outline's points (curves' control points included): left, top, right, bottom.
pub fn bounds(steps: &[Step]) -> Option<(i32, i32, i32, i32)> {
    let mut b: Option<(f32, f32, f32, f32)> = None;
    let mut add = |x: f32, y: f32| {
        b = Some(match b {
            None => (x, y, x, y),
            Some((l, t, r, bo)) => (l.min(x), t.min(y), r.max(x), bo.max(y)),
        });
    };
    for s in steps {
        match *s {
            Step::Move(x, y) | Step::Line(x, y) => add(x, y),
            Step::Quad(a, b, x, y) => {
                add(a, b);
                add(x, y);
            }
            Step::Cubic(a, b, c, d, x, y) => {
                add(a, b);
                add(c, d);
                add(x, y);
            }
            Step::Close => {}
        }
    }
    let (l, t, r, bo) = b?;
    Some((l.floor() as i32, t.floor() as i32, r.ceil() as i32, bo.ceil() as i32)).filter(|&(l, t, r, bo)| r > l && bo > t)
}

/// Coverage of a `width` by `height` grid whose top left is at (`left`, `top`) in the outline's pixels: a byte a pixel,
/// 0 to 255.
pub fn fill(steps: &[Step], left: i32, top: i32, width: usize, height: usize) -> Vec<u8> {
    let mut acc = vec![0f32; width * height + 1];
    let (ox, oy) = (left as f32, top as f32);
    let mut start = (0f32, 0f32);
    let mut at = (0f32, 0f32);
    let line = |acc: &mut Vec<f32>, from: (f32, f32), to: (f32, f32)| add_line(acc, width, height, (from.0 - ox, from.1 - oy), (to.0 - ox, to.1 - oy));
    for s in steps {
        match *s {
            Step::Move(x, y) => {
                if at != start {
                    line(&mut acc, at, start);
                }
                start = (x, y);
                at = start;
            }
            Step::Line(x, y) => {
                line(&mut acc, at, (x, y));
                at = (x, y);
            }
            Step::Quad(cx, cy, x, y) => {
                let n = segments(at, (x, y), &[(cx, cy)]);
                let p0 = at;
                for i in 1..=n {
                    let t = i as f32 / n as f32;
                    let u = 1.0 - t;
                    let p = (u * u * p0.0 + 2.0 * u * t * cx + t * t * x, u * u * p0.1 + 2.0 * u * t * cy + t * t * y);
                    line(&mut acc, at, p);
                    at = p;
                }
            }
            Step::Cubic(ax, ay, bx, by, x, y) => {
                let n = segments(at, (x, y), &[(ax, ay), (bx, by)]);
                let p0 = at;
                for i in 1..=n {
                    let t = i as f32 / n as f32;
                    let u = 1.0 - t;
                    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                    let p = (a * p0.0 + b * ax + c * bx + d * x, a * p0.1 + b * ay + c * by + d * y);
                    line(&mut acc, at, p);
                    at = p;
                }
            }
            Step::Close => {
                if at != start {
                    line(&mut acc, at, start);
                }
                at = start;
            }
        }
    }
    if at != start {
        line(&mut acc, at, start);
    }
    let mut sum = 0f32;
    let mut out = vec![0u8; width * height];
    for (i, o) in out.iter_mut().enumerate() {
        sum += acc[i];
        *o = (sum.abs().min(1.0) * 255.0 + 0.5) as u8;
    }
    out
}

/// How many lines a curve is cut into: enough that none strays a quarter of a pixel from it.
fn segments(from: (f32, f32), to: (f32, f32), controls: &[(f32, f32)]) -> usize {
    // How far the control points stray from the chord bounds how far the curve does.
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let len = (dx * dx + dy * dy).sqrt().max(1e-6);
    let stray = controls.iter().map(|c| ((c.0 - from.0) * dy - (c.1 - from.1) * dx).abs() / len).fold(0f32, f32::max);
    ((stray * 4.0).sqrt().ceil() as usize).clamp(1, 64)
}

/// Adds a line's signed area to the cells it crosses: in each row, what's left of the line in each cell, and the rest
/// of its height to the cell after (whose running sum carries it to the row's end).
fn add_line(acc: &mut [f32], width: usize, height: usize, p0: (f32, f32), p1: (f32, f32)) {
    if (p0.1 - p1.1).abs() < f32::EPSILON {
        return;
    }
    let (dir, p0, p1) = if p0.1 < p1.1 { (1.0, p0, p1) } else { (-1.0, p1, p0) };
    let dxdy = (p1.0 - p0.0) / (p1.1 - p0.1);
    let mut x = p0.0;
    let y_top = p0.1.max(0.0);
    if p0.1 < 0.0 {
        x -= p0.1 * dxdy;
    }
    let y_end = p1.1.min(height as f32);
    let mut y = y_top as usize;
    while (y as f32) < y_end {
        let row = y * width;
        let dy = ((y + 1) as f32).min(y_end) - (y as f32).max(y_top);
        let x_next = x + dxdy * dy;
        let d = dy * dir;
        let (x0, x1) = if x < x_next { (x, x_next) } else { (x_next, x) };
        let x0i = x0.floor();
        let x1i = x1.ceil();
        // A cell left of the grid is its first, and one right of it its end, which the running sum carries into the
        // next row: each row's contributions add to nothing, so that's where what's past the right edge cancels.
        let at = |i: f32| row + (i.max(0.0) as usize).min(width);
        if x1i <= x0i + 1.0 {
            // Within one cell: the part right of the line's middle, then the rest into the next cell.
            let xmf = 0.5 * (x0 + x1) - x0i;
            if x0i < 0.0 {
                acc[row] += d;
            } else {
                acc[at(x0i)] += d - d * xmf;
                acc[at(x0i + 1.0)] += d * xmf;
            }
        } else {
            let s = 1.0 / (x1 - x0);
            let x0f = x0 - x0i;
            let a0 = 0.5 * s * (1.0 - x0f) * (1.0 - x0f);
            let x1f = x1 - x1i + 1.0;
            let am = 0.5 * s * x1f * x1f;
            let mut put = |i: f32, v: f32| acc[at(i)] += v;
            put(x0i, d * a0);
            if x1i == x0i + 2.0 {
                put(x0i + 1.0, d * (1.0 - a0 - am));
            } else {
                let a1 = s * (1.5 - x0f);
                put(x0i + 1.0, d * (a1 - a0));
                let mut xi = x0i + 2.0;
                while xi < x1i - 1.0 {
                    put(xi, d * s);
                    xi += 1.0;
                }
                let a2 = a1 + (x1i - x0i - 3.0) * s;
                put(x1i - 1.0, d * (1.0 - a2 - am));
            }
            put(x1i, d * am);
        }
        x = x_next;
        y += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(l: f32, t: f32, r: f32, b: f32) -> Vec<Step> {
        vec![Step::Move(l, t), Step::Line(r, t), Step::Line(r, b), Step::Line(l, b), Step::Close]
    }

    #[test]
    fn a_square_on_pixels_and_between_them() {
        let px = fill(&square(1.0, 1.0, 3.0, 3.0), 0, 0, 4, 4);
        assert_eq!(px, vec![0, 0, 0, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 0, 0, 0]);
        // Half a pixel over: the edges half covered, the corners a quarter.
        let px = fill(&square(0.5, 0.5, 2.5, 2.5), 0, 0, 3, 3);
        assert_eq!(px, vec![64, 128, 64, 128, 255, 128, 64, 128, 64]);
    }

    #[test]
    fn a_hole_wound_the_other_way_and_an_overlap() {
        let mut steps = square(0.0, 0.0, 4.0, 4.0);
        steps.extend([Step::Move(1.0, 1.0), Step::Line(1.0, 3.0), Step::Line(3.0, 3.0), Step::Line(3.0, 1.0), Step::Close]);
        let px = fill(&steps, 0, 0, 4, 4);
        assert_eq!((px[0], px[5], px[6], px[15]), (255, 0, 0, 255));
        let mut both = square(0.0, 0.0, 2.0, 2.0);
        both.extend(square(1.0, 0.0, 3.0, 2.0));
        assert_eq!(fill(&both, 0, 0, 3, 2), vec![255; 6]);
    }

    #[test]
    fn a_curve_and_bounds() {
        // A circle-ish shape of four quadratics, radius 4 around (5, 5): its middle full, its corners empty.
        let steps = vec![
            Step::Move(9.0, 5.0),
            Step::Quad(9.0, 9.0, 5.0, 9.0),
            Step::Quad(1.0, 9.0, 1.0, 5.0),
            Step::Quad(1.0, 1.0, 5.0, 1.0),
            Step::Quad(9.0, 1.0, 9.0, 5.0),
            Step::Close,
        ];
        assert_eq!(bounds(&steps), Some((1, 1, 9, 9)));
        let px = fill(&steps, 1, 1, 8, 8);
        assert_eq!(px[3 * 8 + 3], 255);
        assert!(px[0] < 128 && px[7] < 128);
        let total: u32 = px.iter().map(|&p| u32::from(p)).sum();
        // A quadratic "circle" has area about 3/4 of its box's, between a circle's (0.785) and a diamond's.
        let area = total as f32 / 255.0 / 64.0;
        assert!((0.7..0.85).contains(&area), "{area}");
    }
}
