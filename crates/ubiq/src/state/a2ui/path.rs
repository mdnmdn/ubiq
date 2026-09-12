//! SVG path data, reduced to the five things a painter has to draw.
//!
//! The A2UI basic catalog lets an `Icon` name itself with `{"svgPath": "M4 12 L10 18 …"}` rather
//! than with a name out of a set, and this is what reads one.
//!
//! **This route needs no sanitizer.** A `d` string is a closed grammar of command letters and
//! numbers: it can express a shape and nothing else — no reference, no entity, no second decoder,
//! nowhere to put a URL. That is the whole reason the catalog offers it, and the whole reason this
//! module is short while [`super::svg`] is not.
//!
//! **The parse normalises.** Relative commands become absolute, `H` and `V` become lines, `S` and
//! `T` get the control point they imply spelled out, and an arc becomes cubics — because a painter
//! that had to understand all twenty commands would be twenty places for the same shape to be drawn
//! two ways. What comes out is five variants, all absolute.
//!
//! **The tokenizer is the fiddly half**, and deliberately not a `split_whitespace`. Path data is
//! written by minifiers: `10-5` is two numbers, `1.5.5` is two numbers, `1e-3` is one, and a
//! command letter may be stated once and meant several times.

/// One point in the path's own coordinate space.
///
/// The space is whatever the `viewBox` around it says; nothing here scales, and nothing here knows
/// what a pixel is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct P {
    pub x: f32,
    pub y: f32,
}

impl P {
    /// A point, written the way every construction in this module wants to write one.
    fn at(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// A path, reduced to the five things a painter has to draw.
///
/// Every point is absolute. [`Seg::Cubic`] carries its two control points then its end point, and
/// [`Seg::Quad`] its one control point then its end point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    Move(P),
    Line(P),
    Cubic(P, P, P),
    Quad(P, P),
    Close,
}

/// Absolute segments, or a sentence naming the command that stopped the parse.
///
/// The error is prose rather than a type, for the same reason as everywhere else in this corner:
/// a payload from an agent fails in one place, a line above a picture that is not drawn.
pub fn parse(d: &str) -> Result<Vec<Seg>, String> {
    let mut lexer = Lexer::new(d);
    let mut out: Vec<Seg> = Vec::new();

    // The three pieces of state a path carries between commands: where the pen is, where the
    // current subpath began — which is where `Z` puts the pen back — and what the last curve's
    // trailing control point was, which is what `S` and `T` reflect.
    let mut cur = P::at(0.0, 0.0);
    let mut start = P::at(0.0, 0.0);
    let mut previous: Option<u8> = None;
    let mut cubic_ctrl: Option<P> = None;
    let mut quad_ctrl: Option<P> = None;

    loop {
        lexer.skip_separators();
        let Some(byte) = lexer.peek() else { break };

        let command = if byte.is_ascii_alphabetic() {
            lexer.bump();
            byte
        } else {
            // An implicit repeat: the previous command stated once and meant again.
            match previous {
                Some(command) => command,
                None => {
                    return Err(format!(
                        "the path data has `{}` where a command letter was expected",
                        byte as char
                    ));
                }
            }
        };

        let absolute = command.is_ascii_uppercase();
        let upper = command.to_ascii_uppercase();

        match upper {
            b'M' => {
                let (x, y) = lexer.pair(command)?;
                cur = resolve(absolute, cur, x, y);
                start = cur;
                out.push(Seg::Move(cur));
                // A second pair after a move is a line, which is the one place the repeat is not
                // the letter that was written.
                previous = Some(if absolute { b'L' } else { b'l' });
                cubic_ctrl = None;
                quad_ctrl = None;
            }
            b'L' => {
                let (x, y) = lexer.pair(command)?;
                cur = resolve(absolute, cur, x, y);
                out.push(Seg::Line(cur));
                previous = Some(command);
                cubic_ctrl = None;
                quad_ctrl = None;
            }
            b'H' => {
                let x = lexer.number(command)?;
                cur = P::at(if absolute { x } else { cur.x + x }, cur.y);
                out.push(Seg::Line(cur));
                previous = Some(command);
                cubic_ctrl = None;
                quad_ctrl = None;
            }
            b'V' => {
                let y = lexer.number(command)?;
                cur = P::at(cur.x, if absolute { y } else { cur.y + y });
                out.push(Seg::Line(cur));
                previous = Some(command);
                cubic_ctrl = None;
                quad_ctrl = None;
            }
            b'C' => {
                let (x1, y1) = lexer.pair(command)?;
                let (x2, y2) = lexer.pair(command)?;
                let (x, y) = lexer.pair(command)?;
                let c1 = resolve(absolute, cur, x1, y1);
                let c2 = resolve(absolute, cur, x2, y2);
                cur = resolve(absolute, cur, x, y);
                out.push(Seg::Cubic(c1, c2, cur));
                previous = Some(command);
                cubic_ctrl = Some(c2);
                quad_ctrl = None;
            }
            b'S' => {
                let (x2, y2) = lexer.pair(command)?;
                let (x, y) = lexer.pair(command)?;
                let c1 = reflect(cubic_ctrl, cur);
                let c2 = resolve(absolute, cur, x2, y2);
                cur = resolve(absolute, cur, x, y);
                out.push(Seg::Cubic(c1, c2, cur));
                previous = Some(command);
                cubic_ctrl = Some(c2);
                quad_ctrl = None;
            }
            b'Q' => {
                let (x1, y1) = lexer.pair(command)?;
                let (x, y) = lexer.pair(command)?;
                let c = resolve(absolute, cur, x1, y1);
                cur = resolve(absolute, cur, x, y);
                out.push(Seg::Quad(c, cur));
                previous = Some(command);
                quad_ctrl = Some(c);
                cubic_ctrl = None;
            }
            b'T' => {
                let (x, y) = lexer.pair(command)?;
                let c = reflect(quad_ctrl, cur);
                cur = resolve(absolute, cur, x, y);
                out.push(Seg::Quad(c, cur));
                previous = Some(command);
                quad_ctrl = Some(c);
                cubic_ctrl = None;
            }
            b'A' => {
                let rx = lexer.number(command)?;
                let ry = lexer.number(command)?;
                let rotation = lexer.number(command)?;
                let large = lexer.number(command)? != 0.0;
                let sweep = lexer.number(command)? != 0.0;
                let (x, y) = lexer.pair(command)?;
                let end = resolve(absolute, cur, x, y);
                arc(cur, (rx, ry), rotation, large, sweep, end, &mut out);
                cur = end;
                previous = Some(command);
                cubic_ctrl = None;
                quad_ctrl = None;
            }
            b'Z' => {
                out.push(Seg::Close);
                cur = start;
                // A close takes no numbers, so it cannot repeat implicitly — and saying so here is
                // what stops a stray number after it from spinning the loop forever.
                previous = None;
                cubic_ctrl = None;
                quad_ctrl = None;
            }
            other => {
                return Err(format!(
                    "`{}` is not a path command this parser knows",
                    other as char
                ));
            }
        }
    }

    Ok(out)
}

/// A point stated absolutely, or relative to where the pen is.
fn resolve(absolute: bool, cur: P, x: f32, y: f32) -> P {
    if absolute {
        P::at(x, y)
    } else {
        P::at(cur.x + x, cur.y + y)
    }
}

/// The control point `S` and `T` imply: the previous one mirrored through the current point.
///
/// With no previous curve of the matching kind there is nothing to mirror, and the spec says the
/// control point is the current point itself — a curve that starts out straight.
fn reflect(previous: Option<P>, cur: P) -> P {
    match previous {
        Some(ctrl) => P::at(2.0 * cur.x - ctrl.x, 2.0 * cur.y - ctrl.y),
        None => cur,
    }
}

// ── the tokenizer ───────────────────────────────────────────────────

/// A cursor over the `d` string, reading one number at a time.
struct Lexer<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Lexer<'a> {
    fn new(d: &'a str) -> Self {
        Self {
            bytes: d.as_bytes(),
            at: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn bump(&mut self) {
        self.at += 1;
    }

    /// Skip what separates two numbers — which may be whitespace, a comma, both, or nothing.
    fn skip_separators(&mut self) {
        while let Some(byte) = self.peek() {
            if byte.is_ascii_whitespace() || byte == b',' {
                self.at += 1;
            } else {
                break;
            }
        }
    }

    /// One number, or a refusal naming the command that ran out of them.
    fn number(&mut self, command: u8) -> Result<f32, String> {
        self.skip_separators();
        let start = self.at;

        if matches!(self.peek(), Some(b'+') | Some(b'-')) {
            self.at += 1;
        }
        let mut digits = 0usize;
        let mut point = false;
        while let Some(byte) = self.peek() {
            if byte.is_ascii_digit() {
                digits += 1;
                self.at += 1;
            } else if byte == b'.' && !point {
                // A second point does not continue this number, it starts the next one: `1.5.5`
                // is two numbers, and a minifier writes it that way on purpose.
                point = true;
                self.at += 1;
            } else {
                break;
            }
        }
        if digits == 0 {
            self.at = start;
            return Err(format!(
                "`{}` has too few numbers after it",
                command as char
            ));
        }

        // An exponent only counts when digits actually follow it; otherwise the `e` belongs to
        // whatever comes next and this number ended before it.
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            let mut ahead = self.at + 1;
            if matches!(self.bytes.get(ahead), Some(b'+') | Some(b'-')) {
                ahead += 1;
            }
            let exponent = ahead;
            while matches!(self.bytes.get(ahead), Some(byte) if byte.is_ascii_digit()) {
                ahead += 1;
            }
            if ahead > exponent {
                self.at = ahead;
            }
        }

        let text = std::str::from_utf8(&self.bytes[start..self.at]).unwrap_or("");
        text.parse::<f32>().map_err(|_| {
            format!(
                "`{}` is followed by `{text}`, which is not a number",
                command as char
            )
        })
    }

    /// Two numbers, which is what most commands take at a time.
    fn pair(&mut self, command: u8) -> Result<(f32, f32), String> {
        Ok((self.number(command)?, self.number(command)?))
    }
}

// ── arcs ────────────────────────────────────────────────────────────

/// Push an arc onto the output as cubics.
///
/// The conversion is the endpoint-to-centre parameterisation the SVG specification gives in its
/// implementation notes, then the sweep split into pieces of no more than a quarter turn, each of
/// which a cubic approximates to well under a rasterised pixel. Both degenerate cases the
/// specification calls out are handled here: a zero radius is a straight line, and radii too small
/// to reach between the endpoints are scaled up until they just do.
fn arc(
    cur: P,
    radii: (f32, f32),
    rotation: f32,
    large: bool,
    sweep: bool,
    end: P,
    out: &mut Vec<Seg>,
) {
    let (mut rx, mut ry) = (radii.0.abs(), radii.1.abs());
    if rx == 0.0 || ry == 0.0 || (cur.x == end.x && cur.y == end.y) {
        if cur.x != end.x || cur.y != end.y {
            out.push(Seg::Line(end));
        }
        return;
    }

    let phi = rotation.to_radians();
    let (sin_phi, cos_phi) = phi.sin_cos();

    let dx = (cur.x - end.x) / 2.0;
    let dy = (cur.y - end.y) / 2.0;
    let x1 = cos_phi * dx + sin_phi * dy;
    let y1 = -sin_phi * dx + cos_phi * dy;

    let spread = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if spread > 1.0 {
        let scale = spread.sqrt();
        rx *= scale;
        ry *= scale;
    }

    let denominator = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let numerator = (rx * rx * ry * ry - denominator).max(0.0);
    let sign = if large == sweep { -1.0 } else { 1.0 };
    let factor = sign * (numerator / denominator).sqrt();
    let cxp = factor * rx * y1 / ry;
    let cyp = -factor * ry * x1 / rx;

    let cx = cos_phi * cxp - sin_phi * cyp + (cur.x + end.x) / 2.0;
    let cy = sin_phi * cxp + cos_phi * cyp + (cur.y + end.y) / 2.0;

    let from = ((x1 - cxp) / rx, (y1 - cyp) / ry);
    let to = ((-x1 - cxp) / rx, (-y1 - cyp) / ry);
    let theta = angle((1.0, 0.0), from);
    let mut delta = angle(from, to);
    if !sweep && delta > 0.0 {
        delta -= std::f32::consts::TAU;
    } else if sweep && delta < 0.0 {
        delta += std::f32::consts::TAU;
    }

    let pieces = (delta.abs() / std::f32::consts::FRAC_PI_2).ceil().max(1.0);
    let step = delta / pieces;
    let k = 4.0 / 3.0 * (step / 4.0).tan();

    for piece in 0..pieces as usize {
        let a = theta + step * piece as f32;
        let b = a + step;
        let (p1, d1) = on_ellipse(cx, cy, rx, ry, sin_phi, cos_phi, a);
        let (p2, d2) = on_ellipse(cx, cy, rx, ry, sin_phi, cos_phi, b);
        out.push(Seg::Cubic(
            P::at(p1.x + k * d1.x, p1.y + k * d1.y),
            P::at(p2.x - k * d2.x, p2.y - k * d2.y),
            p2,
        ));
    }
}

/// A point on the rotated ellipse at one angle, and the tangent there.
fn on_ellipse(cx: f32, cy: f32, rx: f32, ry: f32, sin_phi: f32, cos_phi: f32, at: f32) -> (P, P) {
    let (sin_at, cos_at) = at.sin_cos();
    let x = rx * cos_at;
    let y = ry * sin_at;
    let dx = -rx * sin_at;
    let dy = ry * cos_at;
    (
        P::at(
            cx + cos_phi * x - sin_phi * y,
            cy + sin_phi * x + cos_phi * y,
        ),
        P::at(cos_phi * dx - sin_phi * dy, sin_phi * dx + cos_phi * dy),
    )
}

/// The signed angle from one vector to another.
fn angle(u: (f32, f32), v: (f32, f32)) -> f32 {
    let dot = u.0 * v.0 + u.1 * v.1;
    let lengths = (u.0 * u.0 + u.1 * u.1).sqrt() * (v.0 * v.0 + v.1 * v.1).sqrt();
    let cosine = (dot / lengths).clamp(-1.0, 1.0);
    let sign = if u.0 * v.1 - u.1 * v.0 < 0.0 {
        -1.0
    } else {
        1.0
    };
    sign * cosine.acos()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How close two coordinates have to be to count as the same point.
    ///
    /// Arcs go through trigonometry in `f32`, so exact equality is the wrong assertion even where
    /// the arithmetic is right.
    const EPSILON: f32 = 1e-3;

    fn near(got: P, want: P) {
        assert!(
            (got.x - want.x).abs() < EPSILON && (got.y - want.y).abs() < EPSILON,
            "{got:?} is not near {want:?}"
        );
    }

    #[test]
    fn every_command_absolute() {
        let segments = parse("M0 0 L1 0 H2 V3 C3 4 4 4 5 4 S6 3 6 2 Q6 1 5 1 T3 1 Z")
            .expect("a path using every absolute command");
        assert_eq!(segments[0], Seg::Move(P::at(0.0, 0.0)));
        assert_eq!(segments[1], Seg::Line(P::at(1.0, 0.0)));
        assert_eq!(segments[2], Seg::Line(P::at(2.0, 0.0)));
        assert_eq!(segments[3], Seg::Line(P::at(2.0, 3.0)));
        assert_eq!(
            segments[4],
            Seg::Cubic(P::at(3.0, 4.0), P::at(4.0, 4.0), P::at(5.0, 4.0))
        );
        assert_eq!(segments[6], Seg::Quad(P::at(6.0, 1.0), P::at(5.0, 1.0)));
        assert_eq!(*segments.last().expect("a close"), Seg::Close);
    }

    #[test]
    fn every_command_relative() {
        let segments =
            parse("m1 1 l1 0 h1 v1 c0 1 1 1 1 1 s1 -1 1 -1 q0 -1 -1 -1 t-2 0 z").expect("relative");
        assert_eq!(segments[0], Seg::Move(P::at(1.0, 1.0)));
        assert_eq!(segments[1], Seg::Line(P::at(2.0, 1.0)));
        assert_eq!(segments[2], Seg::Line(P::at(3.0, 1.0)));
        assert_eq!(segments[3], Seg::Line(P::at(3.0, 2.0)));
        assert_eq!(
            segments[4],
            Seg::Cubic(P::at(3.0, 3.0), P::at(4.0, 3.0), P::at(4.0, 3.0))
        );
        assert_eq!(*segments.last().expect("a close"), Seg::Close);
    }

    #[test]
    fn a_sign_separates_two_numbers() {
        assert_eq!(
            parse("M10-5").expect("a minified pair"),
            vec![Seg::Move(P::at(10.0, -5.0))]
        );
    }

    #[test]
    fn a_second_point_starts_the_next_number() {
        assert_eq!(
            parse("M1.5.5").expect("two numbers sharing a point"),
            vec![Seg::Move(P::at(1.5, 0.5))]
        );
    }

    #[test]
    fn exponents_are_one_number_each() {
        assert_eq!(
            parse("M1e-3 1E3").expect("exponents"),
            vec![Seg::Move(P::at(0.001, 1000.0))]
        );
    }

    #[test]
    fn separators_may_be_commas_spaces_or_nothing() {
        let commas = parse("M 0,0 L 1,1").expect("commas");
        let spaces = parse("M0 0L1 1").expect("no separator before the letter");
        assert_eq!(commas, spaces);
    }

    #[test]
    fn a_repeated_group_repeats_the_command() {
        // A move's repeat is a line, and a line's repeat is itself.
        let segments = parse("M0 0 1 1 2 2 L3 3 4 4").expect("implicit repeats");
        assert_eq!(
            segments,
            vec![
                Seg::Move(P::at(0.0, 0.0)),
                Seg::Line(P::at(1.0, 1.0)),
                Seg::Line(P::at(2.0, 2.0)),
                Seg::Line(P::at(3.0, 3.0)),
                Seg::Line(P::at(4.0, 4.0)),
            ]
        );
    }

    #[test]
    fn smooth_curves_reflect_the_previous_control_point() {
        let cubic = parse("M0 0 C1 1 2 2 3 3 S5 5 6 6").expect("a smooth cubic");
        assert_eq!(
            cubic[2],
            Seg::Cubic(P::at(4.0, 4.0), P::at(5.0, 5.0), P::at(6.0, 6.0))
        );

        let quad = parse("M0 0 Q1 1 2 2 T4 4").expect("a smooth quadratic");
        assert_eq!(quad[2], Seg::Quad(P::at(3.0, 3.0), P::at(4.0, 4.0)));
    }

    #[test]
    fn a_quarter_circle_arc_becomes_one_cubic_that_lands_where_it_should() {
        let segments = parse("M1 0 A1 1 0 0 1 0 1").expect("a quarter turn");
        assert_eq!(segments.len(), 2, "a quarter turn needs no splitting");
        let Seg::Cubic(c1, _, end) = segments[1] else {
            panic!("an arc becomes a cubic, got {:?}", segments[1]);
        };
        near(end, P::at(0.0, 1.0));
        // The first control point leaves the start tangentially, which for a circle at (1, 0)
        // means straight along +y by the usual 0.5523 of the radius.
        near(c1, P::at(1.0, 0.55228));
    }

    #[test]
    fn a_half_circle_arc_is_split() {
        let segments = parse("M1 0 A1 1 0 1 1 -1 0").expect("a half turn");
        assert_eq!(segments.len(), 3, "a half turn is two pieces");
        let Seg::Cubic(_, _, end) = *segments.last().expect("a piece") else {
            panic!("an arc becomes cubics");
        };
        near(end, P::at(-1.0, 0.0));
    }

    #[test]
    fn a_zero_radius_arc_is_a_straight_line() {
        assert_eq!(
            parse("M0 0 A0 0 0 0 1 10 10").expect("a degenerate arc"),
            vec![Seg::Move(P::at(0.0, 0.0)), Seg::Line(P::at(10.0, 10.0))]
        );
    }

    #[test]
    fn radii_too_small_are_scaled_until_they_reach() {
        let segments = parse("M0 0 A1 1 0 0 1 10 0").expect("radii that cannot span the endpoints");
        let Seg::Cubic(_, _, end) = *segments.last().expect("a piece") else {
            panic!("an arc becomes cubics");
        };
        near(end, P::at(10.0, 0.0));
    }

    #[test]
    fn close_returns_the_pen_to_the_last_move() {
        let segments = parse("M10 10 L20 20 Z l5 5").expect("a relative line after a close");
        assert_eq!(
            *segments.last().expect("a line"),
            Seg::Line(P::at(15.0, 15.0))
        );
    }

    #[test]
    fn an_unknown_letter_is_named_in_the_refusal() {
        let error = parse("M0 0 X1 1").expect_err("an unknown command");
        assert!(error.contains('X'), "{error}");
    }

    #[test]
    fn a_group_with_too_few_numbers_is_named_in_the_refusal() {
        let error = parse("M0 0 L5").expect_err("half a line");
        assert!(error.contains('L'), "{error}");
    }

    /// The parser's coverage stated as a fact: these are the `d` strings of three icons that ship
    /// in `assets/icons/`, one of them built almost entirely out of arcs.
    #[test]
    fn the_icons_this_repository_ships_parse() {
        // assets/icons/annotate-text.svg — lines only.
        let text = [
            "M12 4v16",
            "M4 7V5a1 1 0 0 1 1-1h14a1 1 0 0 1 1 1v2",
            "M9 20h6",
        ];
        // assets/icons/annotate-crop.svg — a line-and-corner shape with small arcs.
        let crop = ["M6 2v14a2 2 0 0 0 2 2h14", "M18 22V8a2 2 0 0 0-2-2H2"];
        // assets/icons/family-connectors.svg — the arc-heavy one.
        let connectors = [
            "M10.5 13.5a4.5 4.5 0 0 0 6.79.49l2.5-2.5a4.5 4.5 0 0 0-6.36-6.36l-1.44 1.43",
            "M13.5 10.5a4.5 4.5 0 0 0-6.79-.49l-2.5 2.5a4.5 4.5 0 0 0 6.36 6.36l1.43-1.43",
        ];

        for d in text.iter().chain(&crop).chain(&connectors) {
            let segments = parse(d).unwrap_or_else(|error| panic!("`{d}`: {error}"));
            assert!(matches!(segments.first(), Some(Seg::Move(_))), "`{d}`");
            assert!(segments.len() > 1, "`{d}` drew nothing");
        }

        // The corner of `annotate-crop`: a move, a vertical line, one arc, a horizontal line.
        let corner = parse(crop[0]).expect("a corner");
        assert_eq!(corner[0], Seg::Move(P::at(6.0, 2.0)));
        assert_eq!(corner[1], Seg::Line(P::at(6.0, 16.0)));
        assert!(matches!(corner[2], Seg::Cubic(..)));
        assert_eq!(
            *corner.last().expect("a line"),
            Seg::Line(P::at(22.0, 18.0))
        );
    }
}
