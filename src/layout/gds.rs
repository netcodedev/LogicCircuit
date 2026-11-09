//! Minimal GDSII stream reader: structures with their BOUNDARY, BOX and PATH
//! shapes, enough to read the geometry of standard cells.

use std::collections::HashMap;

/// One shape on a GDS layer/datatype, as a closed polygon in database units.
#[derive(Debug, Clone, PartialEq)]
pub struct GdsShape {
    pub layer: i16,
    pub datatype: i16,
    pub points: Vec<(i32, i32)>,
}

#[derive(Debug, Clone, Default)]
pub struct GdsLibrary {
    /// Metres per database unit (1e-9 for sky130: 1 nm).
    pub meters_per_dbu: f64,
    pub structures: HashMap<String, Vec<GdsShape>>,
    /// References to other structures (SREF/AREF) that were not expanded.
    pub skipped_refs: usize,
}

const UNITS: u8 = 0x03;
const BGNSTR: u8 = 0x05;
const STRNAME: u8 = 0x06;
const ENDSTR: u8 = 0x07;
const BOUNDARY: u8 = 0x08;
const PATH: u8 = 0x09;
const SREF: u8 = 0x0A;
const AREF: u8 = 0x0B;
const LAYER: u8 = 0x0D;
const DATATYPE: u8 = 0x0E;
const WIDTH: u8 = 0x0F;
const XY: u8 = 0x10;
const ENDEL: u8 = 0x11;
const BOX: u8 = 0x2D;
const BOXTYPE: u8 = 0x2E;

/// Decodes an 8-byte GDS real (excess-64, base-16 exponent).
pub fn gds_real(b: &[u8]) -> f64 {
    let sign = if b[0] & 0x80 != 0 { -1.0 } else { 1.0 };
    let exp = (b[0] & 0x7F) as i32 - 64;
    let mut mantissa = 0u64;
    for &byte in &b[1..8] {
        mantissa = (mantissa << 8) | byte as u64;
    }
    sign * (mantissa as f64 / 2f64.powi(56)) * 16f64.powi(exp)
}

pub fn parse(data: &[u8]) -> Result<GdsLibrary, String> {
    let mut lib = GdsLibrary::default();
    let mut pos = 0;
    let mut current: Option<(String, Vec<GdsShape>)> = None;
    // Element being read: kind, layer, datatype, width, points.
    let mut element: Option<(u8, i16, i16, i32, Vec<(i32, i32)>)> = None;

    while pos + 4 <= data.len() {
        let len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        if len < 4 || pos + len > data.len() {
            break; // zero padding at the end of the stream
        }
        let rtype = data[pos + 2];
        let payload = &data[pos + 4..pos + len];
        pos += len;
        let i16_at = |i: usize| i16::from_be_bytes([payload[i], payload[i + 1]]);
        let i32_at = |i: usize| i32::from_be_bytes([payload[i], payload[i + 1], payload[i + 2], payload[i + 3]]);

        match rtype {
            UNITS if payload.len() >= 16 => lib.meters_per_dbu = gds_real(&payload[8..16]),
            BGNSTR => current = Some((String::new(), Vec::new())),
            STRNAME => {
                if let Some((name, _)) = current.as_mut() {
                    *name = String::from_utf8_lossy(payload).trim_end_matches('\0').to_string();
                }
            }
            ENDSTR => {
                if let Some((name, shapes)) = current.take() {
                    lib.structures.insert(name, shapes);
                }
            }
            BOUNDARY | PATH | BOX => element = Some((rtype, 0, 0, 0, Vec::new())),
            SREF | AREF => {
                lib.skipped_refs += 1;
                element = None;
            }
            LAYER if payload.len() >= 2 => {
                if let Some(e) = element.as_mut() {
                    e.1 = i16_at(0);
                }
            }
            DATATYPE | BOXTYPE if payload.len() >= 2 => {
                if let Some(e) = element.as_mut() {
                    e.2 = i16_at(0);
                }
            }
            WIDTH if payload.len() >= 4 => {
                if let Some(e) = element.as_mut() {
                    e.3 = i32_at(0).abs();
                }
            }
            XY => {
                if let Some(e) = element.as_mut() {
                    e.4 = (0..payload.len() / 8).map(|i| (i32_at(i * 8), i32_at(i * 8 + 4))).collect();
                }
            }
            ENDEL => {
                if let (Some((kind, layer, datatype, width, pts)), Some((_, shapes))) = (element.take(), current.as_mut()) {
                    let points = match kind {
                        PATH => path_outline(&pts, width),
                        _ => {
                            let mut p = pts;
                            if p.len() > 1 && p.first() == p.last() {
                                p.pop(); // GDS repeats the first point at the end
                            }
                            p
                        }
                    };
                    if points.len() >= 3 {
                        shapes.push(GdsShape { layer, datatype, points });
                    }
                }
            }
            _ => {}
        }
    }
    if lib.structures.is_empty() {
        return Err("no GDS structures found".into());
    }
    Ok(lib)
}

/// Outline of a two-point Manhattan PATH (flush ends). Other paths are
/// approximated by the bounding box of their centre line, widened.
fn path_outline(pts: &[(i32, i32)], width: i32) -> Vec<(i32, i32)> {
    if pts.is_empty() {
        return Vec::new();
    }
    let h = width / 2;
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for &(x, y) in pts {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    if x0 == x1 {
        (x0 -= h, x1 += h);
    } else if y0 == y1 {
        (y0 -= h, y1 += h);
    } else {
        (x0 -= h, y0 -= h, x1 += h, y1 += h);
    }
    vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(t: u8, payload: &[u8]) -> Vec<u8> {
        let mut v = ((payload.len() + 4) as u16).to_be_bytes().to_vec();
        v.extend([t, 0]);
        v.extend(payload);
        v
    }

    #[test]
    fn decodes_reals() {
        // 1e-9 as a GDS real
        let b = [0x39, 0x44, 0xB8, 0x2F, 0xA0, 0x9B, 0x5A, 0x54];
        assert!((gds_real(&b) - 1e-9).abs() < 1e-20);
        assert_eq!(gds_real(&[0x41, 0x10, 0, 0, 0, 0, 0, 0]), 1.0);
    }

    #[test]
    fn reads_boundaries_and_paths() {
        let mut d = Vec::new();
        d.extend(rec(BGNSTR, &[0; 24]));
        d.extend(rec(STRNAME, b"cell"));
        d.extend(rec(BOUNDARY, &[]));
        d.extend(rec(LAYER, &68i16.to_be_bytes()));
        d.extend(rec(DATATYPE, &20i16.to_be_bytes()));
        let pts: Vec<u8> = [(0, 0), (10, 0), (10, 5), (0, 5), (0, 0)]
            .iter()
            .flat_map(|&(x, y): &(i32, i32)| [x.to_be_bytes(), y.to_be_bytes()].concat())
            .collect();
        d.extend(rec(XY, &pts));
        d.extend(rec(ENDEL, &[]));
        d.extend(rec(PATH, &[]));
        d.extend(rec(LAYER, &67i16.to_be_bytes()));
        d.extend(rec(WIDTH, &4i32.to_be_bytes()));
        let line: Vec<u8> = [(0, 10), (20, 10)].iter().flat_map(|&(x, y): &(i32, i32)| [x.to_be_bytes(), y.to_be_bytes()].concat()).collect();
        d.extend(rec(XY, &line));
        d.extend(rec(ENDEL, &[]));
        d.extend(rec(ENDSTR, &[]));
        let lib = parse(&d).unwrap();
        let shapes = &lib.structures["cell"];
        assert_eq!(shapes[0].points, vec![(0, 0), (10, 0), (10, 5), (0, 5)]);
        assert_eq!((shapes[0].layer, shapes[0].datatype), (68, 20));
        assert_eq!(shapes[1].points, vec![(0, 8), (20, 8), (20, 12), (0, 12)]);
    }
}
