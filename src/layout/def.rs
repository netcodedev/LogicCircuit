//! DEF reader for routed OpenROAD output: die, generated vias, components,
//! pins, special nets (power grid) and nets with their routing, expanded to
//! rectangles in database units.

use std::collections::HashMap;

use super::lef::LayerRect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orient {
    N,
    S,
    E,
    W,
    FN,
    FS,
    FE,
    FW,
}

impl Orient {
    pub fn parse(s: &str) -> Orient {
        match s {
            "S" => Orient::S,
            "E" => Orient::E,
            "W" => Orient::W,
            "FN" => Orient::FN,
            "FS" => Orient::FS,
            "FE" => Orient::FE,
            "FW" => Orient::FW,
            _ => Orient::N,
        }
    }

    pub fn code(self) -> i32 {
        self as i32
    }

    /// Maps a point of a cell of size (w, h) into the placed cell, whose
    /// lower-left corner is at the origin (DEF placement semantics).
    pub fn apply(self, (x, y): (i32, i32), w: i32, h: i32) -> (i32, i32) {
        match self {
            Orient::N => (x, y),
            Orient::S => (w - x, h - y),
            Orient::FN => (w - x, y),
            Orient::FS => (x, h - y),
            Orient::W => (h - y, x),
            Orient::E => (y, w - x),
            Orient::FW => (y, x),
            Orient::FE => (h - y, w - x),
        }
    }

    /// Like `apply`, for a rectangle.
    pub fn apply_rect(self, (x0, y0, x1, y1): (i32, i32, i32, i32), w: i32, h: i32) -> (i32, i32, i32, i32) {
        let (ax, ay) = self.apply((x0, y0), w, h);
        let (bx, by) = self.apply((x1, y1), w, h);
        (ax.min(bx), ay.min(by), ax.max(bx), ay.max(by))
    }
}

#[derive(Debug, Clone)]
pub struct DefComponent {
    pub name: String,
    pub master: String,
    pub x: i32,
    pub y: i32,
    pub orient: Orient,
}

#[derive(Debug, Clone)]
pub struct DefPin {
    pub name: String,
    pub net: String,
    pub rects: Vec<LayerRect>,
}

#[derive(Debug, Clone, Default)]
pub struct DefNet {
    pub name: String,
    pub special: bool,
    /// (instance, pin); instance "PIN" for a top-level pin.
    pub connections: Vec<(String, String)>,
    pub shapes: Vec<LayerRect>,
}

#[derive(Debug, Clone, Default)]
pub struct Def {
    pub dbu_per_micron: i32,
    pub die: (i32, i32, i32, i32),
    pub components: Vec<DefComponent>,
    pub pins: Vec<DefPin>,
    pub nets: Vec<DefNet>,
    /// Via names that were referenced but not defined anywhere.
    pub unknown_vias: Vec<String>,
}

/// Geometry of a via generated from a VIARULE (cut array plus enclosures),
/// centred on the origin.
pub fn via_from_rule(params: &HashMap<String, Vec<String>>) -> Vec<LayerRect> {
    let num = |k: &str, i: usize| -> i32 { params.get(k).and_then(|v| v.get(i)).and_then(|s| s.parse().ok()).unwrap_or(0) };
    let layers = params.get("LAYERS").cloned().unwrap_or_default();
    if layers.len() < 3 {
        return Vec::new();
    }
    let (cw, ch) = (num("CUTSIZE", 0), num("CUTSIZE", 1));
    let (sx, sy) = (num("CUTSPACING", 0), num("CUTSPACING", 1));
    let rows = num("ROWCOL", 0).max(1);
    let cols = num("ROWCOL", 1).max(1);
    let (ox, oy) = (num("ORIGIN", 0), num("ORIGIN", 1));
    let w = cols * cw + (cols - 1) * sx;
    let h = rows * ch + (rows - 1) * sy;
    let (x0, y0) = (ox - w / 2, oy - h / 2);
    let mut rects = Vec::new();
    rects.push((layers[0].clone(), x0 - num("ENCLOSURE", 0), y0 - num("ENCLOSURE", 1), x0 + w + num("ENCLOSURE", 0), y0 + h + num("ENCLOSURE", 1)));
    rects.push((layers[2].clone(), x0 - num("ENCLOSURE", 2), y0 - num("ENCLOSURE", 3), x0 + w + num("ENCLOSURE", 2), y0 + h + num("ENCLOSURE", 3)));
    for r in 0..rows {
        for c in 0..cols {
            let cx = x0 + c * (cw + sx);
            let cy = y0 + r * (ch + sy);
            rects.push((layers[1].clone(), cx, cy, cx + cw, cy + ch));
        }
    }
    rects
}

fn tokens(text: &str) -> Vec<&str> {
    text.split_whitespace().collect()
}

/// Parses a routed DEF. `lef_vias` are the fixed vias of the tech LEF and
/// `widths` the default routing width per layer (DBU).
pub fn parse(text: &str, lef_vias: &HashMap<String, Vec<LayerRect>>, widths: &HashMap<String, i32>) -> Result<Def, String> {
    let t = tokens(text);
    let mut def = Def { dbu_per_micron: 1000, ..Default::default() };
    let mut vias: HashMap<String, Vec<LayerRect>> = lef_vias.clone();
    let mut ndr: HashMap<String, HashMap<String, i32>> = HashMap::new();
    let num = |s: &str| -> i32 { s.parse().unwrap_or(0) };
    let mut i = 0;
    let n = t.len();

    while i < n {
        match t[i] {
            // "END VIAS" etc.: skip the section name, or it would start the section again.
            "END" => i += 2,
            "UNITS" if i + 3 < n => {
                def.dbu_per_micron = num(t[i + 3]);
                i += 4;
            }
            "DIEAREA" if i + 9 < n => {
                def.die = (num(t[i + 2]), num(t[i + 3]), num(t[i + 6]), num(t[i + 7]));
                i += 9;
            }
            "VIAS" => {
                i += 3; // VIAS n ;
                while i < n && t[i] != "END" {
                    if t[i] == "-" {
                        let name = t[i + 1].to_string();
                        i += 2;
                        let mut params: HashMap<String, Vec<String>> = HashMap::new();
                        let mut rects = Vec::new();
                        while i < n && t[i] != ";" {
                            if t[i] == "+" && i + 1 < n {
                                let key = t[i + 1];
                                i += 2;
                                if key == "RECT" || key == "POLYGON" {
                                    // + RECT layer ( x0 y0 ) ( x1 y1 )
                                    let layer = t[i].to_string();
                                    let v: Vec<i32> = t[i + 1..].iter().take_while(|s| **s != "+" && **s != ";").filter_map(|s| s.parse().ok()).collect();
                                    if v.len() >= 4 {
                                        rects.push((layer, v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])));
                                    }
                                    continue;
                                }
                                let vals: Vec<String> = t[i..].iter().take_while(|s| **s != "+" && **s != ";").map(|s| s.to_string()).collect();
                                i += vals.len();
                                params.insert(key.to_string(), vals);
                            } else {
                                i += 1;
                            }
                        }
                        if params.contains_key("VIARULE") {
                            rects.extend(via_from_rule(&params));
                        }
                        vias.insert(name, rects);
                    }
                    i += 1;
                }
            }
            "NONDEFAULTRULES" => {
                i += 3;
                let mut current = String::new();
                while i < n && !(t[i] == "END" && t.get(i + 1) == Some(&"NONDEFAULTRULES")) {
                    if t[i] == "-" {
                        current = t[i + 1].to_string();
                    } else if t[i] == "LAYER" && t.get(i + 2) == Some(&"WIDTH") {
                        ndr.entry(current.clone()).or_default().insert(t[i + 1].to_string(), num(t[i + 3]));
                    }
                    i += 1;
                }
            }
            "COMPONENTS" => {
                i += 3;
                while i < n && t[i] != "END" {
                    if t[i] == "-" {
                        let (name, master) = (t[i + 1].to_string(), t[i + 2].to_string());
                        i += 3;
                        let mut placed = None;
                        while i < n && t[i] != ";" {
                            if (t[i] == "PLACED" || t[i] == "FIXED") && i + 5 < n {
                                placed = Some((num(t[i + 2]), num(t[i + 3]), Orient::parse(t[i + 5])));
                                i += 6;
                            } else {
                                i += 1;
                            }
                        }
                        if let Some((x, y, orient)) = placed {
                            def.components.push(DefComponent { name, master, x, y, orient });
                        }
                    }
                    i += 1;
                }
            }
            "PINS" => {
                i += 3;
                while i < n && t[i] != "END" {
                    if t[i] == "-" {
                        let mut pin = DefPin { name: t[i + 1].to_string(), net: String::new(), rects: Vec::new() };
                        let mut local: Vec<LayerRect> = Vec::new();
                        let mut at = (0, 0, Orient::N);
                        i += 2;
                        while i < n && t[i] != ";" {
                            match t[i] {
                                "NET" => {
                                    pin.net = t[i + 1].to_string();
                                    i += 2;
                                }
                                "LAYER" if i + 9 < n => {
                                    local.push((t[i + 1].to_string(), num(t[i + 3]), num(t[i + 4]), num(t[i + 7]), num(t[i + 8])));
                                    i += 10;
                                }
                                "PLACED" | "FIXED" if i + 5 < n => {
                                    at = (num(t[i + 2]), num(t[i + 3]), Orient::parse(t[i + 5]));
                                    i += 6;
                                }
                                _ => i += 1,
                            }
                        }
                        for (layer, x0, y0, x1, y1) in local {
                            // Pin shapes are relative to the pin location; rotate about it.
                            let (a0, b0, a1, b1) = at.2.apply_rect((x0, y0, x1, y1), 0, 0);
                            pin.rects.push((layer, at.0 + a0, at.1 + b0, at.0 + a1, at.1 + b1));
                        }
                        def.pins.push(pin);
                    }
                    i += 1;
                }
            }
            section @ ("SPECIALNETS" | "NETS") => {
                let special = section == "SPECIALNETS";
                i += 3;
                while i < n && !(t[i] == "END" && t.get(i + 1) == Some(&section)) {
                    if t[i] != "-" {
                        i += 1;
                        continue;
                    }
                    let mut net = DefNet { name: t[i + 1].to_string(), special, ..Default::default() };
                    let mut net_widths = widths.clone();
                    i += 2;
                    // Connections: ( inst pin ) ...
                    while i + 3 < n && t[i] == "(" {
                        net.connections.push((t[i + 1].to_string(), t[i + 2].to_string()));
                        i += 4;
                    }
                    while i < n && t[i] != ";" {
                        if t[i] == "+" && i + 1 < n {
                            match t[i + 1] {
                                "NONDEFAULTRULE" if i + 2 < n => {
                                    if let Some(rule) = ndr.get(t[i + 2]) {
                                        net_widths.extend(rule.iter().map(|(k, v)| (k.clone(), *v)));
                                    }
                                    i += 3;
                                }
                                "ROUTED" | "FIXED" | "COVER" => {
                                    i = parse_routing(&t, i + 2, special, &net_widths, &vias, &mut net, &mut def.unknown_vias);
                                }
                                _ => i += 2,
                            }
                        } else {
                            i += 1;
                        }
                    }
                    def.nets.push(net);
                    i += 1;
                }
                i += 2;
            }
            _ => i += 1,
        }
    }
    def.unknown_vias.sort();
    def.unknown_vias.dedup();
    Ok(def)
}

/// Parses routing statements (`layer [width] [+ SHAPE ..] points/vias`,
/// separated by NEW) starting at `i`; returns the index of the token that
/// ends them (`;` or a `+` of the next net attribute).
fn parse_routing(
    t: &[&str],
    mut i: usize,
    special: bool,
    widths: &HashMap<String, i32>,
    vias: &HashMap<String, Vec<LayerRect>>,
    net: &mut DefNet,
    unknown: &mut Vec<String>,
) -> usize {
    let n = t.len();
    'statements: while i < n {
        let layer = t[i].to_string();
        i += 1;
        let mut width = widths.get(&layer).copied().unwrap_or(0);
        if special && i < n && t[i].parse::<i32>().is_ok() {
            width = t[i].parse().unwrap();
            i += 1;
        }
        while i + 2 < n && t[i] == "+" && matches!(t[i + 1], "SHAPE" | "STYLE" | "MASK") {
            i += 3;
        }
        if i < n && t[i] == "TAPER" {
            i += 1;
        } else if i + 1 < n && t[i] == "TAPERRULE" {
            i += 2;
        }
        let half = width / 2;
        let mut last: Option<(i32, i32, Option<i32>)> = None;
        while i < n {
            match t[i] {
                "(" => {
                    // ( x y [ext] ) with '*' = previous coordinate
                    let mut vals = Vec::new();
                    i += 1;
                    while i < n && t[i] != ")" {
                        vals.push(t[i]);
                        i += 1;
                    }
                    i += 1;
                    let prev = last.map(|p| (p.0, p.1)).unwrap_or((0, 0));
                    let x = if vals.first() == Some(&"*") { prev.0 } else { vals.first().and_then(|v| v.parse().ok()).unwrap_or(prev.0) };
                    let y = if vals.get(1) == Some(&"*") { prev.1 } else { vals.get(1).and_then(|v| v.parse().ok()).unwrap_or(prev.1) };
                    let ext = vals.get(2).and_then(|v| v.parse::<i32>().ok());
                    if let Some((px, py, pext)) = last {
                        // Regular wires extend half a width past their ends
                        // unless an explicit extension is given; special wires are flush.
                        let default_ext = if special { 0 } else { half };
                        let (e0, e1) = (pext.unwrap_or(default_ext), ext.unwrap_or(default_ext));
                        let rect = if py == y {
                            let (a, ea, b, eb) = if px <= x { (px, e0, x, e1) } else { (x, e1, px, e0) };
                            (a - ea, y - half, b + eb, y + half)
                        } else {
                            let (a, ea, b, eb) = if py <= y { (py, e0, y, e1) } else { (y, e1, py, e0) };
                            (x - half, a - ea, x + half, b + eb)
                        };
                        net.shapes.push((layer.clone(), rect.0, rect.1, rect.2, rect.3));
                    }
                    last = Some((x, y, ext));
                }
                "RECT" if i + 5 < n => {
                    // RECT ( dx0 dy0 dx1 dy1 ) relative to the last point
                    let v: Vec<i32> = t[i + 2..i + 6].iter().filter_map(|s| s.parse().ok()).collect();
                    if let (Some((x, y, _)), 4) = (last, v.len()) {
                        net.shapes.push((layer.clone(), x + v[0].min(v[2]), y + v[1].min(v[3]), x + v[0].max(v[2]), y + v[1].max(v[3])));
                    }
                    i += 7;
                }
                "MASK" => i += 2,
                "NEW" => {
                    i += 1;
                    continue 'statements;
                }
                ";" | "+" => return i,
                via => {
                    if let Some((x, y, _)) = last {
                        match vias.get(via) {
                            Some(rects) => {
                                net.shapes.extend(rects.iter().map(|(l, x0, y0, x1, y1)| (l.clone(), x + x0, y + y0, x + x1, y + y1)));
                            }
                            None => unknown.push(via.to_string()),
                        }
                    }
                    i += 1;
                }
            }
        }
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEF: &str = "
VERSION 5.8 ;
UNITS DISTANCE MICRONS 1000 ;
DIEAREA ( 0 0 ) ( 10000 8000 ) ;
VIAS 1 ;
    - v12 + VIARULE M1M2_PR + CUTSIZE 150 150  + LAYERS met1 via met2  + CUTSPACING 170 170  + ENCLOSURE 85 165 55 85  + ROWCOL 1 2  ;
END VIAS
NONDEFAULTRULES 1 ;
    - CTS_NDR_0
      + LAYER met1 WIDTH 200 SPACING 400
END NONDEFAULTRULES
COMPONENTS 1 ;
    - u_a sky130_fd_sc_hd__inv_1 + SOURCE DIST + PLACED ( 1000 2720 ) FS ;
END COMPONENTS
PINS 1 ;
    - A + NET A + DIRECTION INPUT + USE SIGNAL
      + PORT
        + LAYER met3 ( -400 -150 ) ( 400 150 )
        + PLACED ( 9600 4000 ) N ;
END PINS
SPECIALNETS 1 ;
    - VDD ( * VPWR ) + USE POWER
      + ROUTED met1 480 + SHAPE FOLLOWPIN ( 0 2720 ) ( 10000 2720 )
      NEW met2 0 + SHAPE STRIPE ( 5000 2720 ) v12 ;
END SPECIALNETS
NETS 1 ;
    - A ( PIN A ) ( u_a A ) + USE SIGNAL + NONDEFAULTRULE CTS_NDR_0
      + ROUTED met1 ( 1000 3000 ) ( 2000 * 0 )
      NEW met1 ( 2000 3000 ) RECT ( 0 -70 255 70 )
      NEW met2 ( 1500 3000 ) ( * 4000 ) v12 ;
END NETS
END DESIGN
";

    #[test]
    fn parses_routed_def() {
        let mut widths = HashMap::new();
        widths.insert("met1".to_string(), 140);
        widths.insert("met2".to_string(), 140);
        let def = parse(DEF, &HashMap::new(), &widths).unwrap();
        assert_eq!(def.die, (0, 0, 10000, 8000));
        assert_eq!(def.components[0].orient, Orient::FS);
        assert_eq!(def.pins[0].rects[0], ("met3".to_string(), 9200, 3850, 10000, 4150));
        let vdd = &def.nets[0];
        assert!(vdd.special);
        // flush special wire, width 480
        assert_eq!(vdd.shapes[0], ("met1".to_string(), 0, 2480, 10000, 2960));
        // generated via: 2 cuts of 150 with 170 spacing = 470 wide
        assert!(vdd.shapes.iter().any(|s| s.0 == "via" && s.1 == 5000 - 235));
        let a = &def.nets[1];
        assert_eq!(a.connections, vec![("PIN".to_string(), "A".to_string()), ("u_a".to_string(), "A".to_string())]);
        // NDR width 200, start extended by 100, end extension 0
        assert_eq!(a.shapes[0], ("met1".to_string(), 900, 2900, 2000, 3100));
        assert_eq!(a.shapes[1], ("met1".to_string(), 2000, 2930, 2255, 3070));
        assert_eq!(a.shapes[2], ("met2".to_string(), 1430, 2930, 1570, 4070));
        assert!(def.unknown_vias.is_empty());
    }

    #[test]
    fn orientations_keep_cells_in_place() {
        // A 100x50 cell: its corners stay within the placed box for every orientation.
        for o in [Orient::N, Orient::S, Orient::FN, Orient::FS] {
            assert_eq!(o.apply_rect((0, 0, 100, 50), 100, 50), (0, 0, 100, 50));
        }
        assert_eq!(Orient::FS.apply((10, 5), 100, 50), (10, 45));
        assert_eq!(Orient::W.apply_rect((0, 0, 100, 50), 100, 50), (0, 0, 50, 100));
    }
}
