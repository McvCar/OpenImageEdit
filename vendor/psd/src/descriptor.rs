//! Bounds-checked reader for Photoshop *descriptor* structures.
//!
//! Descriptors are Photoshop's generic key/value tree. They carry layer styles
//! (`lfx2`), vector shape data (`vscg`) and most of the 16.0-era metadata. The
//! format is self-describing but has no length prefix of its own, so a single
//! misread byte desynchronises everything after it. Every read here therefore
//! returns `Option` and a parse failure discards the whole descriptor instead
//! of producing half-decoded garbage.
//!
//! # Format
//!
//! ```text
//! descriptor body:
//!   4   unicode-string length (characters)
//!   2n  UTF-16BE name          (NOT padded, unlike `luni`)
//!   -   class id: either  4-byte length 0 + 4 raw bytes, or length + bytes
//!   4   item count
//!   -   items: 4-byte key + 4-byte OS type + value
//! ```

/// One value inside a descriptor.
#[derive(Debug, Clone, PartialEq)]
pub enum DescriptorValue {
    /// `bool`
    Bool(bool),
    /// `long`
    Integer(i32),
    /// `doub`
    Double(f64),
    /// `UntF` / `UnFl`: a double plus a four-character unit ("#Pxl", "#Prc").
    Unit {
        /// Unit key, e.g. `b"#Pxl"`.
        unit: [u8; 4],
        /// Numeric value.
        value: f64,
    },
    /// `TEXT`
    Text(String),
    /// `enum`: a type id and a value, both four-character keys.
    Enum {
        /// Type id, e.g. `b"BlnM"`.
        type_id: [u8; 4],
        /// Value, e.g. `b"Mltp"`.
        value: [u8; 4],
    },
    /// `Objc` / `GlbO` / `type`
    Descriptor(Descriptor),
    /// `VlLs`
    List(Vec<DescriptorValue>),
    /// A value we deliberately do not model (`alis`, `Pth `, `tdta`, ...).
    Opaque,
}

/// A descriptor: an ordered list of `(key, value)` pairs plus a class id.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Descriptor {
    /// Class id of the object (e.g. `b"null"` or `b"Lefx"`).
    pub class_id: [u8; 4],
    /// Items in file order. Keys are usually four ASCII characters, but the
    /// format allows longer ones (`layerConceals`), so they are stored whole.
    /// Lookups keep the first match, like Photoshop.
    pub items: Vec<(Vec<u8>, DescriptorValue)>,
}

impl Descriptor {
    /// First value stored under `key`.
    pub fn get(&self, key: &[u8]) -> Option<&DescriptorValue> {
        self.items.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Boolean value, `None` when absent or of another type.
    pub fn boolean(&self, key: &[u8]) -> Option<bool> {
        match self.get(key)? {
            DescriptorValue::Bool(b) => Some(*b),
            DescriptorValue::Integer(i) => Some(*i != 0),
            _ => None,
        }
    }

    /// Any numeric value (integer, double or unit) as `f64`.
    pub fn number(&self, key: &[u8]) -> Option<f64> {
        match self.get(key)? {
            DescriptorValue::Integer(i) => Some(*i as f64),
            DescriptorValue::Double(d) => Some(*d),
            DescriptorValue::Unit { value, .. } => Some(*value),
            DescriptorValue::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    /// Enum *value* (the right-hand four characters), e.g. `b"Mltp"`.
    pub fn enum_value(&self, key: &[u8]) -> Option<[u8; 4]> {
        match self.get(key)? {
            DescriptorValue::Enum { value, .. } => Some(*value),
            _ => None,
        }
    }

    /// Nested descriptor.
    pub fn descriptor(&self, key: &[u8]) -> Option<&Descriptor> {
        match self.get(key)? {
            DescriptorValue::Descriptor(d) => Some(d),
            _ => None,
        }
    }

    /// List value.
    pub fn list(&self, key: &[u8]) -> Option<&[DescriptorValue]> {
        match self.get(key)? {
            DescriptorValue::List(l) => Some(l.as_slice()),
            _ => None,
        }
    }

    /// Text value.
    pub fn text(&self, key: &[u8]) -> Option<&str> {
        match self.get(key)? {
            DescriptorValue::Text(t) => Some(t.as_str()),
            _ => None,
        }
    }
}

/// Cursor over a byte slice. All reads are bounds-checked and return `None`
/// rather than panicking: the vendored parser runs inside a host process.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Reader { bytes, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let slice = self.bytes.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn u32(&mut self) -> Option<u32> {
        let b = self.take(4)?;
        Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn i32(&mut self) -> Option<i32> {
        let b = self.take(4)?;
        Some(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn i64(&mut self) -> Option<i64> {
        let b = self.take(8)?;
        let mut v = [0u8; 8];
        v.copy_from_slice(b);
        Some(i64::from_be_bytes(v))
    }

    fn f64(&mut self) -> Option<f64> {
        let b = self.take(8)?;
        let mut v = [0u8; 8];
        v.copy_from_slice(b);
        Some(f64::from_be_bytes(v))
    }

    fn key(&mut self) -> Option<[u8; 4]> {
        let b = self.take(4)?;
        Some([b[0], b[1], b[2], b[3]])
    }

    /// Four-byte key used as a descriptor item key.
    fn key_vec(&mut self) -> Option<Vec<u8>> {
        Some(self.key()?.to_vec())
    }

    /// Four-character key, or a length-prefixed longer one.
    fn length_and_key(&mut self) -> Option<Vec<u8>> {
        let len = self.u32()? as usize;
        if len == 0 {
            return Some(self.key()?.to_vec());
        }
        // A key longer than this is certainly a misread length.
        if len > 1024 {
            return None;
        }
        Some(self.take(len)?.to_vec())
    }

    fn unicode_string(&mut self) -> Option<String> {
        let chars = self.u32()? as usize;
        // A corrupt length must not make us allocate gigabytes.
        if chars > 65_535 {
            return None;
        }
        let raw = self.take(chars.checked_mul(2)?)?;
        let units: Vec<u16> = raw
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        Some(String::from_utf16_lossy(&units))
    }
}

/// Parse a descriptor *body* (no version prefix). `key` is `b"Objc"`-style.
pub fn parse_body(bytes: &[u8]) -> Option<Descriptor> {
    let mut r = Reader::new(bytes);
    parse_descriptor_body(&mut r)
}

/// Parse a `DescriptorBlock2` payload: `version`, `data_version`, then body.
///
/// This is the layout of the `lfx2` (layer styles) block: two four-byte
/// versions in front of the descriptor body.
pub fn parse_block2(bytes: &[u8]) -> Option<Descriptor> {
    let mut r = Reader::new(bytes);
    let _version = r.u32()?;
    let _data_version = r.u32()?;
    parse_descriptor_body(&mut r)
}

fn dbg_on() -> bool {
    std::env::var_os("IMAGEEDIT_DEBUG_DESCRIPTOR").is_some()
}

fn parse_descriptor_body(r: &mut Reader) -> Option<Descriptor> {
    if dbg_on() {
        eprintln!("  body at {}", r.pos);
    }
    let _name = r.unicode_string()?;
    let class_key = r.length_and_key()?;
    let mut class_id = [0u8; 4];
    for (i, slot) in class_id.iter_mut().enumerate() {
        *slot = *class_key.get(i).unwrap_or(&0);
    }
    let count = r.u32()? as usize;
    if count > 4096 {
        return None;
    }
    let mut items = Vec::with_capacity(count.min(256));
    for _ in 0..count {
        let at = r.pos;
        let key = r.length_and_key()?;
        let ostype = r.key()?;
        if dbg_on() {
            eprintln!(
                "    item @{at} key {:?} type {:?}",
                String::from_utf8_lossy(&key),
                String::from_utf8_lossy(&ostype)
            );
        }
        let value = parse_value(r, ostype)?;
        items.push((key, value));
    }
    Some(Descriptor { class_id, items })
}

/// Value prefixed by its four-character OS type.
fn parse_typed_value(r: &mut Reader) -> Option<DescriptorValue> {
    let os_type = r.key()?;
    parse_value(r, os_type)
}

/// Truncate/pad a variable-length key to the four characters we compare on.
fn four(key: Vec<u8>) -> [u8; 4] {
    let mut out = [0u8; 4];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = *key.get(i).unwrap_or(&0);
    }
    out
}

fn parse_value(r: &mut Reader, os_type: [u8; 4]) -> Option<DescriptorValue> {
    Some(match &os_type {
        b"Objc" | b"GlbO" | b"type" => DescriptorValue::Descriptor(parse_descriptor_body(r)?),
        b"VlLs" => {
            let count = r.u32()? as usize;
            if count > 65_535 {
                return None;
            }
            let mut items = Vec::with_capacity(count.min(256));
            for _ in 0..count {
                items.push(parse_typed_value(r)?);
            }
            DescriptorValue::List(items)
        }
        b"doub" => DescriptorValue::Double(r.f64()?),
        b"UntF" | b"UnFl" => {
            let unit = r.key()?;
            DescriptorValue::Unit {
                unit,
                value: r.f64()?,
            }
        }
        b"TEXT" => DescriptorValue::Text(r.unicode_string()?),
        // Both halves of an enum are length-prefixed keys (usually a zero
        // length followed by four characters).
        b"enum" => DescriptorValue::Enum {
            type_id: four(r.length_and_key()?),
            value: four(r.length_and_key()?),
        },
        b"long" => DescriptorValue::Integer(r.i32()?),
        b"comp" => DescriptorValue::Double(r.i64()? as f64),
        b"bool" => DescriptorValue::Bool(r.u8()? != 0),
        b"alis" | b"Pth " | b"tdta" => {
            let len = r.u32()? as usize;
            r.take(len)?;
            DescriptorValue::Opaque
        }
        _ => {
            if std::env::var_os("IMAGEEDIT_DEBUG_DESCRIPTOR").is_some() {
                eprintln!(
                    "descriptor: unknown OS type {:?}",
                    String::from_utf8_lossy(&os_type)
                );
            }
            return None;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_u32(v: &mut Vec<u8>, n: u32) {
        v.extend_from_slice(&n.to_be_bytes());
    }

    fn push_ustring(v: &mut Vec<u8>, s: &str) {
        let units: Vec<u16> = s.encode_utf16().collect();
        push_u32(v, units.len() as u32);
        for u in units {
            v.extend_from_slice(&u.to_be_bytes());
        }
    }

    /// `{null { Scl : 100% }}`
    fn simple_descriptor() -> Vec<u8> {
        let mut v = Vec::new();
        push_u32(&mut v, 0); // version
        push_u32(&mut v, 16); // data version
        push_ustring(&mut v, "");
        push_u32(&mut v, 0);
        v.extend_from_slice(b"null");
        push_u32(&mut v, 1);
        push_u32(&mut v, 0);
        v.extend_from_slice(b"Scl ");
        v.extend_from_slice(b"UntF");
        v.extend_from_slice(b"#Prc");
        v.extend_from_slice(&100.0f64.to_be_bytes());
        v
    }

    #[test]
    fn reads_a_block2_descriptor() {
        let d = parse_block2(&simple_descriptor()).expect("parse");
        assert_eq!(d.class_id, *b"null");
        assert_eq!(d.number(b"Scl "), Some(100.0));
    }

    #[test]
    fn truncated_input_is_none_not_a_panic() {
        let full = simple_descriptor();
        for cut in 0..full.len() {
            assert!(parse_block2(&full[..cut]).is_none(), "cut at {cut}");
        }
    }
}
