//! 2D images of a layout for the CLI (`chip render`): a top view of chosen
//! layers, or an isometric view of the layer stack, as SVG or PNG.
//!
//! Both formats share one drawing path: the layout is turned into filled
//! polygons per layer (in microns), projected, and either written as SVG
//! paths or rasterised with a small scanline filler.

use std::fmt::Write as _;

use super::{Layout, MasterShape, SKY130_STACK};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Svg,
    Png,
}

#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// Indices into SKY130_STACK, drawn bottom to top.
    pub layers: Vec<usize>,
    pub iso: bool,
    /// Include the power grid (special nets).
    pub power: bool,
    /// Isometric view: extra microns between consecutive layers.
    pub explode: f64,
    /// Isometric view: vertical exaggeration.
    pub zscale: f64,
    /// Image width in pixels (PNG; SVG uses it for its default size).
    pub width_px: u32,
    pub background: [u8; 3],
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            layers: Vec::new(),
            iso: false,
            power: false,
            explode: 4.0,
            zscale: 4.0,
            width_px: 1600,
            background: [15, 18, 22],
        }
    }
}

/// Default layers: metal routing for the top view, the whole device stack
/// (without wells and contacts) for the isometric view.
pub fn default_layers(iso: bool) -> Vec<usize> {
    let names: &[&str] = if iso {
        &["diff", "poly", "li1", "met1", "met2", "met3", "met4", "met5"]
    } else {
        &["li1", "met1", "met2", "met3", "met4", "met5"]
    };
    names.iter().filter_map(|n| SKY130_STACK.iter().position(|l| l.name == *n)).collect()
}

fn hex_rgb(c: &str) -> [u8; 3] {
    let v = u32::from_str_radix(c.trim_start_matches('#'), 16).unwrap_or(0x808080);
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

/// Polygons (microns) per requested layer, cells placed and oriented.
fn polygons(layout: &Layout, opts: &RenderOptions) -> Vec<(usize, Vec<Vec<(f64, f64)>>)> {
    let dbu = layout.dbu_per_micron as f64;
    let mut per_layer: Vec<Vec<Vec<(f64, f64)>>> = vec![Vec::new(); SKY130_STACK.len()];
    let wanted: Vec<bool> = (0..SKY130_STACK.len()).map(|i| opts.layers.contains(&i)).collect();
    for inst in &layout.instances {
        let m = &layout.masters[inst.master as usize];
        for s in &m.shapes {
            let (layer, pts): (u8, Vec<(i32, i32)>) = match s {
                MasterShape::Rect { layer, x0, y0, x1, y1 } => (*layer, vec![(*x0, *y0), (*x1, *y0), (*x1, *y1), (*x0, *y1)]),
                MasterShape::Polygon { layer, points } => (*layer, points.clone()),
            };
            if !wanted[layer as usize] {
                continue;
            }
            let poly = pts
                .iter()
                .map(|&p| {
                    let (x, y) = inst.orient.apply(p, m.width, m.height);
                    ((inst.x + x) as f64 / dbu, (inst.y + y) as f64 / dbu)
                })
                .collect();
            per_layer[layer as usize].push(poly);
        }
    }
    for r in &layout.routes {
        if !wanted[r.layer as usize] || (!opts.power && layout.nets[r.net as usize].special) {
            continue;
        }
        let (x0, y0, x1, y1) = (r.x0 as f64 / dbu, r.y0 as f64 / dbu, r.x1 as f64 / dbu, r.y1 as f64 / dbu);
        per_layer[r.layer as usize].push(vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]);
    }
    opts.layers.iter().map(|&l| (l, std::mem::take(&mut per_layer[l]))).collect()
}

/// A filled polygon in image coordinates with colour and opacity.
struct Fill {
    points: Vec<(f64, f64)>,
    color: [u8; 3],
    alpha: f64,
}

/// Projects the layout into image-space fills, in drawing order, and returns
/// them with the image size.
fn scene(layout: &Layout, opts: &RenderOptions) -> (Vec<Fill>, f64, f64) {
    let dbu = layout.dbu_per_micron as f64;
    let (dx0, dy0, dx1, dy1) = (
        layout.die.0 as f64 / dbu,
        layout.die.1 as f64 / dbu,
        layout.die.2 as f64 / dbu,
        layout.die.3 as f64 / dbu,
    );
    let layers = polygons(layout, opts);
    let mut fills = Vec::new();
    // Die outline as backdrop.
    let die = vec![(dx0, dy0), (dx1, dy0), (dx1, dy1), (dx0, dy1)];

    if !opts.iso {
        let scale = opts.width_px as f64 / (dx1 - dx0);
        let to_img = |(x, y): (f64, f64)| ((x - dx0) * scale, (dy1 - y) * scale);
        fills.push(Fill { points: die.iter().map(|&p| to_img(p)).collect(), color: [32, 37, 45], alpha: 1.0 });
        for (l, polys) in layers {
            let color = hex_rgb(SKY130_STACK[l].color);
            for p in polys {
                fills.push(Fill { points: p.into_iter().map(to_img).collect(), color, alpha: 0.75 });
            }
        }
        return (fills, opts.width_px as f64, (dy1 - dy0) * scale);
    }

    // Isometric: X = (x - y) cos30, Y = (x + y) sin30 + z (Y up), drawn bottom
    // layer first and, within a layer, back (large x + y) to front.
    let (c, s) = (30f64.to_radians().cos(), 30f64.to_radians().sin());
    let z_of = |l: usize| SKY130_STACK[l].z * opts.zscale + opts.explode * l as f64;
    let top_z = layers.iter().map(|(l, _)| z_of(*l) + SKY130_STACK[*l].thickness * opts.zscale).fold(0.0, f64::max);
    let project = |(x, y): (f64, f64), z: f64| ((x - y) * c, (x + y) * s + z);
    // Bounds of the projected die at the bottom and the top of the stack.
    let corners: Vec<(f64, f64)> = die.iter().flat_map(|&p| [project(p, 0.0), project(p, top_z)]).collect();
    let (min_x, max_x) = corners.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.0), a.1.max(p.0)));
    let (min_y, max_y) = corners.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.1), a.1.max(p.1)));
    let margin = 0.03 * (max_x - min_x);
    let scale = opts.width_px as f64 / (max_x - min_x + 2.0 * margin);
    // Screen Y grows upwards in the projection; images grow downwards.
    let to_img = |(px, py): (f64, f64)| ((px - min_x + margin) * scale, (max_y - py + margin) * scale);
    fills.push(Fill { points: die.iter().map(|&p| to_img(project(p, 0.0))).collect(), color: [32, 37, 45], alpha: 1.0 });
    for (l, mut polys) in layers {
        let base = hex_rgb(SKY130_STACK[l].color);
        let dark = base.map(|v| (v as f64 * 0.55) as u8);
        let (z0, z1) = (z_of(l), z_of(l) + SKY130_STACK[l].thickness * opts.zscale);
        polys.sort_by(|a, b| {
            let ka = a.iter().map(|p| p.0 + p.1).fold(f64::MIN, f64::max);
            let kb = b.iter().map(|p| p.0 + p.1).fold(f64::MIN, f64::max);
            kb.total_cmp(&ka)
        });
        for p in polys {
            // Darker bottom face first gives the plates a visible edge.
            fills.push(Fill { points: p.iter().map(|&q| to_img(project(q, z0))).collect(), color: dark, alpha: 0.9 });
            fills.push(Fill { points: p.iter().map(|&q| to_img(project(q, z1))).collect(), color: base, alpha: 0.9 });
        }
    }
    (fills, opts.width_px as f64, (max_y - min_y + 2.0 * margin) * scale)
}

/// Renders the layout. SVG output groups paths by colour; PNG is
/// rasterised with even-odd scanline filling and alpha blending.
pub fn render(layout: &Layout, opts: &RenderOptions, format: Format) -> Result<Vec<u8>, String> {
    let (fills, w, h) = scene(layout, opts);
    match format {
        Format::Svg => Ok(svg(&fills, w, h, opts.background).into_bytes()),
        Format::Png => png_bytes(&fills, w.ceil() as u32, h.ceil() as u32, opts.background),
    }
}

fn svg(fills: &[Fill], w: f64, h: f64, bg: [u8; 3]) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w:.1} {h:.1}\" width=\"{w:.0}\" height=\"{h:.0}\">"
    );
    let _ = writeln!(out, "<rect width=\"100%\" height=\"100%\" fill=\"rgb({},{},{})\"/>", bg[0], bg[1], bg[2]);
    // Consecutive fills with the same style share one path (keeps order).
    let mut i = 0;
    while i < fills.len() {
        let (color, alpha) = (fills[i].color, fills[i].alpha);
        let mut d = String::new();
        while i < fills.len() && fills[i].color == color && fills[i].alpha == alpha {
            let pts = &fills[i].points;
            let _ = write!(d, "M{:.2} {:.2}", pts[0].0, pts[0].1);
            for p in &pts[1..] {
                let _ = write!(d, "L{:.2} {:.2}", p.0, p.1);
            }
            d.push('Z');
            i += 1;
        }
        let _ = writeln!(
            out,
            "<path fill=\"rgb({},{},{})\" fill-opacity=\"{alpha}\" fill-rule=\"evenodd\" d=\"{d}\"/>",
            color[0], color[1], color[2]
        );
    }
    out.push_str("</svg>\n");
    out
}

/// Fills `fills` into an RGB image (even-odd rule, alpha blending).
pub fn rasterize(fills: &[impl AsFill], w: u32, h: u32, bg: [u8; 3]) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let mut img: Vec<u8> = bg.iter().copied().cycle().take(w * h * 3).collect();
    let mut xs: Vec<f64> = Vec::new();
    for f in fills {
        let pts = f.points();
        let (y_min, y_max) = pts.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.1), a.1.max(p.1)));
        let row0 = (y_min - 0.5).ceil().max(0.0) as usize;
        let row1 = ((y_max - 0.5).floor() as isize).min(h as isize - 1);
        if row1 < row0 as isize {
            // Thinner than a pixel row: still draw one row so fine wires stay visible.
            let y = ((y_min + y_max) / 2.0).floor();
            if y >= 0.0 && (y as usize) < h {
                span_fill(&mut img, w, y as usize, pts, f.color(), f.alpha(), &mut xs, Some(y + 0.5));
            }
            continue;
        }
        for row in row0..=row1 as usize {
            span_fill(&mut img, w, row, pts, f.color(), f.alpha(), &mut xs, None);
        }
    }
    img
}

#[allow(clippy::too_many_arguments)]
fn span_fill(img: &mut [u8], w: usize, row: usize, pts: &[(f64, f64)], color: [u8; 3], alpha: f64, xs: &mut Vec<f64>, yc: Option<f64>) {
    let y = yc.unwrap_or(row as f64 + 0.5);
    xs.clear();
    for k in 0..pts.len() {
        let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
        if (a.1 <= y && b.1 > y) || (b.1 <= y && a.1 > y) {
            xs.push(a.0 + (y - a.1) / (b.1 - a.1) * (b.0 - a.0));
        }
    }
    if xs.is_empty() && yc.is_some() {
        // Degenerate (zero-height) shape: fill its x extent.
        let (x0, x1) = pts.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.0), a.1.max(p.0)));
        xs.extend([x0, x1]);
    }
    xs.sort_by(f64::total_cmp);
    for pair in xs.chunks(2) {
        if pair.len() < 2 {
            break;
        }
        let mut c0 = (pair[0] - 0.5).ceil().max(0.0) as usize;
        let mut c1 = ((pair[1] - 0.5).floor() as isize).min(w as isize - 1);
        if c1 < c0 as isize {
            // Narrower than a pixel: one pixel.
            let x = ((pair[0] + pair[1]) / 2.0).floor();
            if x < 0.0 || x as usize >= w {
                continue;
            }
            c0 = x as usize;
            c1 = c0 as isize;
        }
        for col in c0..=c1 as usize {
            let i = (row * w + col) * 3;
            for ch in 0..3 {
                img[i + ch] = (img[i + ch] as f64 * (1.0 - alpha) + color[ch] as f64 * alpha).round() as u8;
            }
        }
    }
}

/// Access to a fill for the rasteriser (also used by tests).
pub trait AsFill {
    fn points(&self) -> &[(f64, f64)];
    fn color(&self) -> [u8; 3];
    fn alpha(&self) -> f64;
}

impl AsFill for Fill {
    fn points(&self) -> &[(f64, f64)] {
        &self.points
    }
    fn color(&self) -> [u8; 3] {
        self.color
    }
    fn alpha(&self) -> f64 {
        self.alpha
    }
}

fn png_bytes(fills: &[Fill], w: u32, h: u32, bg: [u8; 3]) -> Result<Vec<u8>, String> {
    let img = rasterize(fills, w, h, bg);
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&img).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct F(Vec<(f64, f64)>);
    impl AsFill for F {
        fn points(&self) -> &[(f64, f64)] {
            &self.0
        }
        fn color(&self) -> [u8; 3] {
            [255, 0, 0]
        }
        fn alpha(&self) -> f64 {
            1.0
        }
    }

    #[test]
    fn rasterises_rectangles_and_thin_lines() {
        let fills = [F(vec![(1.0, 1.0), (3.0, 1.0), (3.0, 3.0), (1.0, 3.0)]), F(vec![(0.0, 4.2), (4.0, 4.2), (4.0, 4.4), (0.0, 4.4)])];
        let img = rasterize(&fills, 5, 5, [0, 0, 0]);
        let px = |x: usize, y: usize| img[(y * 5 + x) * 3];
        // 2x2 square covers pixels (1..3, 1..3)
        assert_eq!((px(1, 1), px(2, 2), px(3, 3), px(0, 0)), (255, 255, 0, 0));
        // a 0.2 px high wire still shows up as one row
        assert_eq!(px(2, 4), 255);
    }

    #[test]
    fn default_layers_exist() {
        assert_eq!(default_layers(false).len(), 6);
        assert_eq!(default_layers(true).len(), 8);
    }
}
