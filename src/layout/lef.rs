//! LEF reader for what the layout extraction needs: layer types, default
//! routing widths and thicknesses (tech LEF), fixed via definitions, and cell
//! macros with their size and pin rectangles. Coordinates are converted from
//! microns to database units (nm, `DBU` per micron).

use std::collections::HashMap;

pub const DBU: f64 = 1000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerKind {
    Routing,
    Cut,
    Other,
}

#[derive(Debug, Clone)]
pub struct LefLayer {
    pub name: String,
    pub kind: LayerKind,
    /// Default wire width in DBU (routing layers).
    pub width: i32,
    /// Thickness in microns, if given.
    pub thickness: Option<f64>,
}

/// A rectangle on a named layer, in DBU: (layer, x0, y0, x1, y1).
pub type LayerRect = (String, i32, i32, i32, i32);

#[derive(Debug, Clone)]
pub struct LefPin {
    pub name: String,
    pub rects: Vec<LayerRect>,
}

#[derive(Debug, Clone)]
pub struct LefMacro {
    pub name: String,
    pub width: i32,
    pub height: i32,
    pub pins: Vec<LefPin>,
}

#[derive(Debug, Clone, Default)]
pub struct Lef {
    pub layers: Vec<LefLayer>,
    pub vias: HashMap<String, Vec<LayerRect>>,
    pub macros: HashMap<String, LefMacro>,
}

fn to_dbu(v: &str) -> i32 {
    (v.parse::<f64>().unwrap_or(0.0) * DBU).round() as i32
}

/// Whitespace tokens with comments removed and ';' as its own token.
fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("");
        for tok in line.split_whitespace() {
            match tok.strip_suffix(';') {
                Some(rest) if !rest.is_empty() => {
                    out.push(rest.to_string());
                    out.push(";".into());
                }
                _ => out.push(tok.to_string()),
            }
        }
    }
    out
}

/// Reads LAYER ... RECT x0 y0 x1 y1 ; groups until `END <end>`; returns the
/// rectangles and the index after `END <end>`.
fn read_rects(t: &[String], mut i: usize, end: &str) -> (Vec<LayerRect>, usize) {
    let mut rects = Vec::new();
    let mut layer = String::new();
    while i < t.len() {
        match t[i].as_str() {
            "END" if t.get(i + 1).map(String::as_str) == Some(end) || end.is_empty() => {
                return (rects, i + if end.is_empty() { 1 } else { 2 });
            }
            "LAYER" => {
                layer = t.get(i + 1).cloned().unwrap_or_default();
                i += 2;
            }
            "RECT" if i + 4 < t.len() => {
                // "RECT MASK n x0 y0 x1 y1" is not used by sky130 cells.
                let (x0, y0, x1, y1) = (to_dbu(&t[i + 1]), to_dbu(&t[i + 2]), to_dbu(&t[i + 3]), to_dbu(&t[i + 4]));
                rects.push((layer.clone(), x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)));
                i += 5;
            }
            _ => i += 1,
        }
    }
    (rects, i)
}

/// Parses one or more LEF files' contents into `lef` (later files add to it).
pub fn parse_into(lef: &mut Lef, text: &str) {
    let t = tokens(text);
    let mut i = 0;
    while i < t.len() {
        match t[i].as_str() {
            "LAYER" if t.get(i + 2).map(String::as_str) == Some("TYPE") => {
                // Only top-level LAYER blocks have a TYPE right after the name.
                let name = t[i + 1].clone();
                let mut layer = LefLayer { name: name.clone(), kind: LayerKind::Other, width: 0, thickness: None };
                i += 2;
                while i < t.len() && !(t[i] == "END" && t.get(i + 1) == Some(&name)) {
                    match t[i].as_str() {
                        "TYPE" => {
                            layer.kind = match t.get(i + 1).map(String::as_str) {
                                Some("ROUTING") => LayerKind::Routing,
                                Some("CUT") => LayerKind::Cut,
                                _ => LayerKind::Other,
                            };
                        }
                        "WIDTH" if layer.width == 0 && t.get(i + 2).map(String::as_str) == Some(";") => {
                            layer.width = to_dbu(&t[i + 1]);
                        }
                        "THICKNESS" => layer.thickness = t.get(i + 1).and_then(|v| v.parse().ok()),
                        _ => {}
                    }
                    i += 1;
                }
                i += 2;
                lef.layers.push(layer);
            }
            "VIA" if i + 1 < t.len() => {
                let name = t[i + 1].clone();
                let (rects, next) = read_rects(&t, i + 2, &name);
                lef.vias.insert(name, rects);
                i = next;
            }
            "VIARULE" if i + 1 < t.len() => {
                let name = t[i + 1].clone();
                i += 2;
                while i < t.len() && !(t[i] == "END" && t.get(i + 1) == Some(&name)) {
                    i += 1;
                }
                i += 2;
            }
            "MACRO" if i + 1 < t.len() => {
                let name = t[i + 1].clone();
                let mut m = LefMacro { name: name.clone(), width: 0, height: 0, pins: Vec::new() };
                i += 2;
                while i < t.len() && !(t[i] == "END" && t.get(i + 1) == Some(&name)) {
                    match t[i].as_str() {
                        "SIZE" if i + 3 < t.len() => {
                            m.width = to_dbu(&t[i + 1]);
                            m.height = to_dbu(&t[i + 3]);
                            i += 4;
                        }
                        "PIN" if i + 1 < t.len() => {
                            let pin = t[i + 1].clone();
                            let (rects, next) = read_rects(&t, i + 2, &pin);
                            m.pins.push(LefPin { name: pin, rects });
                            i = next;
                        }
                        "OBS" => {
                            let (_, next) = read_rects(&t, i + 1, "");
                            i = next;
                        }
                        _ => i += 1,
                    }
                }
                i += 2;
                lef.macros.insert(name, m);
            }
            _ => i += 1,
        }
    }
}

impl Lef {
    pub fn layer(&self, name: &str) -> Option<&LefLayer> {
        self.layers.iter().find(|l| l.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TECH: &str = "
LAYER li1
  TYPE ROUTING ;
  WIDTH 0.17 ;          # LI 1
  SPACINGTABLE
     PARALLELRUNLENGTH 0
     WIDTH 0 0.17 ;
  THICKNESS 0.1 ;
END li1
LAYER mcon
  TYPE CUT ;
  WIDTH 0.17 ;
END mcon
VIA L1M1_PR DEFAULT
  LAYER mcon ;
  RECT -0.085 -0.085 0.085 0.085 ;
  LAYER met1 ;
  RECT -0.145 -0.115 0.145 0.115 ;
END L1M1_PR
VIARULE L1M1_PR GENERATE
  LAYER li1 ;
  ENCLOSURE 0 0 ;
END L1M1_PR
MACRO cell
  ORIGIN 0 0 ;
  SIZE 1.38 BY 2.72 ;
  PIN A
    DIRECTION INPUT ;
    PORT
      LAYER li1 ;
        RECT 0.94 1.075 1.275 1.325 ;
    END
  END A
  OBS
    LAYER li1 ;
      RECT 0 0 1 1 ;
  END
END cell
";

    #[test]
    fn parses_layers_vias_and_macros() {
        let mut lef = Lef::default();
        parse_into(&mut lef, TECH);
        let li1 = lef.layer("li1").unwrap();
        assert_eq!((li1.kind, li1.width, li1.thickness), (LayerKind::Routing, 170, Some(0.1)));
        assert_eq!(lef.layer("mcon").unwrap().kind, LayerKind::Cut);
        assert_eq!(lef.vias["L1M1_PR"][1], ("met1".to_string(), -145, -115, 145, 115));
        let m = &lef.macros["cell"];
        assert_eq!((m.width, m.height), (1380, 2720));
        assert_eq!(m.pins[0].rects, vec![("li1".to_string(), 940, 1075, 1275, 1325)]);
    }
}
