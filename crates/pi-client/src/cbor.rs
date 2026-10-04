//! The protocol's strict, definite-length RFC 8949 subset, over `serde_json::Value`.
//!
//! Mirrors Pi's `@earendil-works/pi-protocol` codec: no tags, no indefinite lengths, no
//! half/single floats, string map keys only, no duplicate keys, integers within the JavaScript
//! safe range, finite floats only, bounded size, container length and depth.
//!
//! Byte strings are valid CBOR but not JSON, and every protocol payload must be strict JSON, so
//! the decoder rejects them here rather than returning a value the protocol would reject anyway.

use serde_json::{Map, Number, Value};

use crate::error::{Error, Result};

pub const DEFAULT_MAX_BYTE_LENGTH: usize = 16 * 1024 * 1024;
pub const DEFAULT_MAX_CONTAINER_LENGTH: usize = 1_000_000;
pub const DEFAULT_MAX_DEPTH: usize = 64;

const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Maximum encoded size and maximum text string length, in bytes.
    pub max_byte_length: usize,
    /// Maximum elements in an array or entries in a map.
    pub max_container_length: usize,
    /// Maximum nesting; the top-level item is depth 0.
    pub max_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_byte_length: DEFAULT_MAX_BYTE_LENGTH,
            max_container_length: DEFAULT_MAX_CONTAINER_LENGTH,
            max_depth: DEFAULT_MAX_DEPTH,
        }
    }
}

fn err(message: impl Into<String>) -> Error {
    Error::Cbor(message.into())
}

// ----------------------------------------------------------------------------- encoder

pub fn encode(value: &Value, limits: &Limits) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    encode_value(&mut out, value, limits, 0)?;
    Ok(out)
}

fn check_size(out: &[u8], limits: &Limits) -> Result<()> {
    if out.len() > limits.max_byte_length {
        return Err(err(format!(
            "CBOR byte length exceeds configured limit of {}",
            limits.max_byte_length
        )));
    }
    Ok(())
}

fn write_argument(out: &mut Vec<u8>, major: u8, value: u64) {
    let prefix = major << 5;
    if value < 24 {
        out.push(prefix | value as u8);
    } else if value <= 0xff {
        out.push(prefix | 24);
        out.push(value as u8);
    } else if value <= 0xffff {
        out.push(prefix | 25);
        out.extend_from_slice(&(value as u16).to_be_bytes());
    } else if value <= 0xffff_ffff {
        out.push(prefix | 26);
        out.extend_from_slice(&(value as u32).to_be_bytes());
    } else {
        out.push(prefix | 27);
        out.extend_from_slice(&value.to_be_bytes());
    }
}

fn encode_text(out: &mut Vec<u8>, text: &str, limits: &Limits) -> Result<()> {
    if text.len() > limits.max_byte_length {
        return Err(err(format!(
            "CBOR text string length exceeds configured limit of {}",
            limits.max_byte_length
        )));
    }
    write_argument(out, 3, text.len() as u64);
    out.extend_from_slice(text.as_bytes());
    Ok(())
}

fn encode_value(out: &mut Vec<u8>, value: &Value, limits: &Limits, depth: usize) -> Result<()> {
    if depth > limits.max_depth {
        return Err(err(format!(
            "CBOR nesting depth exceeds configured limit of {}",
            limits.max_depth
        )));
    }
    match value {
        Value::Null => out.push(0xf6),
        Value::Bool(true) => out.push(0xf5),
        Value::Bool(false) => out.push(0xf4),
        Value::Number(n) => encode_number(out, n)?,
        Value::String(s) => encode_text(out, s, limits)?,
        Value::Array(items) => {
            if items.len() > limits.max_container_length {
                return Err(err(format!(
                    "CBOR array length exceeds configured limit of {}",
                    limits.max_container_length
                )));
            }
            write_argument(out, 4, items.len() as u64);
            for item in items {
                encode_value(out, item, limits, depth + 1)?;
            }
        }
        Value::Object(map) => {
            if map.len() > limits.max_container_length {
                return Err(err(format!(
                    "CBOR map length exceeds configured limit of {}",
                    limits.max_container_length
                )));
            }
            write_argument(out, 5, map.len() as u64);
            for (key, item) in map {
                encode_text(out, key, limits)?;
                encode_value(out, item, limits, depth + 1)?;
            }
        }
    }
    check_size(out, limits)
}

fn encode_number(out: &mut Vec<u8>, n: &Number) -> Result<()> {
    if let Some(u) = n.as_u64() {
        if u > MAX_SAFE_INTEGER {
            return Err(err("CBOR integers must be safe JavaScript integers"));
        }
        write_argument(out, 0, u);
    } else if let Some(i) = n.as_i64() {
        // Here `i` is negative: non-negative values were handled as u64.
        let magnitude = i.unsigned_abs();
        if magnitude > MAX_SAFE_INTEGER {
            return Err(err("CBOR integers must be safe JavaScript integers"));
        }
        write_argument(out, 1, magnitude - 1);
    } else {
        let f = n
            .as_f64()
            .ok_or_else(|| err("CBOR numbers must be finite"))?;
        if !f.is_finite() {
            return Err(err("CBOR numbers must be finite"));
        }
        // Integral floats are integers, as in JavaScript, except negative zero.
        if f.fract() == 0.0 && !(f == 0.0 && f.is_sign_negative()) {
            if f.abs() > MAX_SAFE_INTEGER as f64 {
                return Err(err("CBOR integers must be safe JavaScript integers"));
            }
            if f >= 0.0 {
                write_argument(out, 0, f as u64);
            } else {
                write_argument(out, 1, (-1.0 - f) as u64);
            }
        } else {
            out.push(0xfb);
            out.extend_from_slice(&f.to_be_bytes());
        }
    }
    Ok(())
}

// ----------------------------------------------------------------------------- decoder

pub fn decode(bytes: &[u8], limits: &Limits) -> Result<Value> {
    if bytes.len() > limits.max_byte_length {
        return Err(err(format!(
            "CBOR byte length exceeds configured limit of {}",
            limits.max_byte_length
        )));
    }
    let mut reader = Reader {
        bytes,
        offset: 0,
        limits,
    };
    let value = reader.item(0)?;
    if reader.offset != bytes.len() {
        return Err(err("CBOR payload contains trailing data"));
    }
    Ok(value)
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
    limits: &'a Limits,
}

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8> {
        let b = *self
            .bytes
            .get(self.offset)
            .ok_or_else(|| err("Truncated CBOR payload"))?;
        self.offset += 1;
        Ok(b)
    }

    fn take(&mut self, n: usize) -> Result<&[u8]> {
        if n > self.bytes.len() - self.offset {
            return Err(err("Truncated CBOR payload"));
        }
        let slice = &self.bytes[self.offset..self.offset + n];
        self.offset += n;
        Ok(slice)
    }

    fn argument(&mut self, info: u8) -> Result<u64> {
        match info {
            0..=23 => Ok(info as u64),
            24 => Ok(self.byte()? as u64),
            25 => Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()) as u64),
            26 => Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()) as u64),
            27 => {
                let value = u64::from_be_bytes(self.take(8)?.try_into().unwrap());
                if value > MAX_SAFE_INTEGER {
                    return Err(err(
                        "Decoded CBOR integer or length is outside the safe range",
                    ));
                }
                Ok(value)
            }
            31 => Err(err("Indefinite-length CBOR items are not supported")),
            _ => Err(err("Malformed CBOR additional information")),
        }
    }

    fn length(&mut self, info: u8, kind: &str, limit: usize) -> Result<usize> {
        if info == 31 {
            return Err(err(format!(
                "Indefinite-length CBOR {kind}s are not supported"
            )));
        }
        let length = self.argument(info)?;
        if length > limit as u64 {
            return Err(err(format!(
                "CBOR {kind} length exceeds configured limit of {limit}"
            )));
        }
        Ok(length as usize)
    }

    fn item(&mut self, depth: usize) -> Result<Value> {
        if depth > self.limits.max_depth {
            return Err(err(format!(
                "CBOR nesting depth exceeds configured limit of {}",
                self.limits.max_depth
            )));
        }
        let initial = self.byte()?;
        let major = initial >> 5;
        let info = initial & 0x1f;
        match major {
            0 => Ok(Value::Number(Number::from(self.argument(info)?))),
            1 => {
                let argument = self.argument(info)?;
                if argument > MAX_SAFE_INTEGER - 1 {
                    return Err(err("Decoded CBOR integer is outside the safe range"));
                }
                Ok(Value::Number(Number::from(-1 - argument as i64)))
            }
            2 => {
                // Consume it faithfully so the failure is about type, not truncation.
                let n = self.length(info, "byte string", self.limits.max_byte_length)?;
                self.take(n)?;
                Err(err("CBOR byte strings are not valid JSON"))
            }
            3 => {
                let n = self.length(info, "text string", self.limits.max_byte_length)?;
                let raw = self.take(n)?;
                let text = std::str::from_utf8(raw)
                    .map_err(|_| err("CBOR text string contains invalid UTF-8"))?;
                Ok(Value::String(text.to_owned()))
            }
            4 => {
                let n = self.length(info, "array", self.limits.max_container_length)?;
                // Never trust a declared length for allocation.
                let mut items = Vec::new();
                for _ in 0..n {
                    items.push(self.item(depth + 1)?);
                }
                Ok(Value::Array(items))
            }
            5 => {
                let n = self.length(info, "map", self.limits.max_container_length)?;
                let mut map = Map::new();
                for _ in 0..n {
                    let key = match self.item(depth + 1)? {
                        Value::String(s) => s,
                        _ => return Err(err("CBOR map keys must be strings")),
                    };
                    if map.contains_key(&key) {
                        return Err(err("CBOR map contains a duplicate key"));
                    }
                    let value = self.item(depth + 1)?;
                    map.insert(key, value);
                }
                Ok(Value::Object(map))
            }
            6 => Err(err("CBOR tags are not supported")),
            _ => self.simple(info),
        }
    }

    fn simple(&mut self, info: u8) -> Result<Value> {
        match info {
            20 => Ok(Value::Bool(false)),
            21 => Ok(Value::Bool(true)),
            22 => Ok(Value::Null),
            27 => {
                let f = f64::from_be_bytes(self.take(8)?.try_into().unwrap());
                if !f.is_finite() {
                    return Err(err("Decoded CBOR number must be finite"));
                }
                if f.fract() == 0.0 && f.abs() > MAX_SAFE_INTEGER as f64 {
                    return Err(err("Decoded CBOR integer is outside the safe range"));
                }
                // An integral float is the same JSON number as the integer, except -0.
                if f.fract() == 0.0 && !(f == 0.0 && f.is_sign_negative()) {
                    let n = if f >= 0.0 {
                        Number::from(f as u64)
                    } else {
                        Number::from(f as i64)
                    };
                    return Ok(Value::Number(n));
                }
                Ok(Value::Number(Number::from_f64(f).ok_or_else(|| {
                    err("Decoded CBOR number must be finite")
                })?))
            }
            31 => Err(err("CBOR break marker is not supported")),
            _ => Err(err("Unsupported CBOR simple value or floating-point width")),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn from_hex(hex: &str) -> Vec<u8> {
        assert!(hex.len().is_multiple_of(2));
        (0..hex.len() / 2)
            .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap())
            .collect()
    }

    fn to_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn limits() -> Limits {
        Limits::default()
    }

    /// Vectors from Pi's `protocol/test/cbor/cbor.test.ts`.
    #[test]
    fn known_vectors_round_trip() {
        let vectors: Vec<(Value, &str)> = vec![
            (json!(null), "f6"),
            (json!(false), "f4"),
            (json!(true), "f5"),
            (json!(0), "00"),
            (json!(1), "01"),
            (json!(10), "0a"),
            (json!(23), "17"),
            (json!(24), "1818"),
            (json!(25), "1819"),
            (json!(100), "1864"),
            (json!(1000), "1903e8"),
            (json!(1_000_000), "1a000f4240"),
            (json!(1_000_000_000_000u64), "1b000000e8d4a51000"),
            (json!(9_007_199_254_740_991u64), "1b001fffffffffffff"),
            (json!(-1), "20"),
            (json!(-10), "29"),
            (json!(-24), "37"),
            (json!(-25), "3818"),
            (json!(-100), "3863"),
            (json!(-1000), "3903e7"),
            (json!(-1_000_000), "3a000f423f"),
            (json!(-9_007_199_254_740_991i64), "3b001ffffffffffffe"),
            (json!(1.1), "fb3ff199999999999a"),
            (json!(""), "60"),
            (json!("IETF"), "6449455446"),
            (json!("\u{fc}"), "62c3bc"),
            (json!("\u{6c34}"), "63e6b0b4"),
            (json!("\u{10151}"), "64f0908591"),
            (json!([]), "80"),
            (json!([1, 2, 3]), "83010203"),
            (json!([1, [2, 3], [4, 5]]), "8301820203820405"),
            (json!({"a": 1, "b": [2, 3]}), "a26161016162820203"),
        ];
        for (value, wire) in vectors {
            assert_eq!(to_hex(&encode(&value, &limits()).unwrap()), wire, "{value}");
            assert_eq!(decode(&from_hex(wire), &limits()).unwrap(), value, "{wire}");
        }
    }

    #[test]
    fn negative_zero_is_a_float_and_survives() {
        let negative_zero = Value::Number(Number::from_f64(-0.0).unwrap());
        assert_eq!(
            to_hex(&encode(&negative_zero, &limits()).unwrap()),
            "fb8000000000000000"
        );
        let decoded = decode(&from_hex("fb8000000000000000"), &limits()).unwrap();
        assert!(decoded.as_f64().unwrap().is_sign_negative());
    }

    #[test]
    fn integral_float64_decodes_as_an_integer() {
        // 3.0 as float64 is the JSON number 3, as it is in JavaScript.
        assert_eq!(
            decode(&from_hex("fb4008000000000000"), &limits()).unwrap(),
            json!(3)
        );
        // ...and an integral float re-encodes as an integer.
        let three = Value::Number(Number::from_f64(3.0).unwrap());
        assert_eq!(to_hex(&encode(&three, &limits()).unwrap()), "03");
    }

    #[test]
    fn preserves_a_leading_bom_and_treats_proto_as_data() {
        assert_eq!(
            decode(&from_hex("63efbbbf"), &limits()).unwrap(),
            json!("\u{feff}")
        );
        let value = json!({"__proto__": "safe"});
        let decoded = decode(&encode(&value, &limits()).unwrap(), &limits()).unwrap();
        assert_eq!(decoded["__proto__"], "safe");
    }

    #[test]
    fn rejects_invalid_decoder_input() {
        let cases = [
            ("empty input", ""),
            ("truncated integer", "18"),
            ("reserved additional information", "1c"),
            ("indefinite byte string", "5f"),
            ("indefinite text string", "7f"),
            ("indefinite array", "9f"),
            ("indefinite map", "bf"),
            ("tag", "c000"),
            ("undefined", "f7"),
            ("unsupported simple value", "e0"),
            ("break outside an indefinite item", "ff"),
            ("float16", "f93c00"),
            ("float32", "fa3f800000"),
            ("positive infinity", "fb7ff0000000000000"),
            ("NaN", "fb7ff8000000000000"),
            ("truncated float64", "fb3ff00000"),
            ("truncated byte string", "44010203"),
            ("truncated text string", "636162"),
            ("truncated array", "8201"),
            ("truncated map", "a16161"),
            ("trailing data", "0000"),
            ("non-string map key", "a10102"),
            ("duplicate map key", "a2616101616102"),
            ("invalid UTF-8 byte", "61ff"),
            ("overlong UTF-8", "62c080"),
            ("UTF-8 surrogate", "63eda080"),
            ("unsafe positive integer", "1b0020000000000000"),
            ("unsafe negative integer", "3b001fffffffffffff"),
            ("unsafe integer encoded as float64", "fb4340000000000000"),
            ("byte string is not JSON", "4401020304"),
        ];
        for (label, wire) in cases {
            let result = decode(&from_hex(wire), &limits());
            assert!(matches!(result, Err(Error::Cbor(_))), "{label}: {result:?}");
        }
    }

    #[test]
    fn rejects_unsupported_encoder_values() {
        assert!(encode(&json!(9_007_199_254_740_992u64), &limits()).is_err());
        assert!(encode(&json!(-9_007_199_254_740_992i64), &limits()).is_err());
        let huge = Value::Number(Number::from_f64(1e300).unwrap());
        assert!(encode(&huge, &limits()).is_err());
    }

    #[test]
    fn depth_is_bounded_in_both_directions() {
        let mut too_deep = json!(null);
        for _ in 0..=DEFAULT_MAX_DEPTH {
            too_deep = json!([too_deep]);
        }
        assert!(
            encode(&too_deep, &limits())
                .unwrap_err()
                .to_string()
                .contains("depth")
        );
        let mut ok = json!(null);
        for _ in 0..DEFAULT_MAX_DEPTH {
            ok = json!([ok]);
        }
        let bytes = encode(&ok, &limits()).unwrap();
        assert_eq!(decode(&bytes, &limits()).unwrap(), ok);

        // 65 nested arrays around a null puts the null at depth 65.
        let mut wire = vec![0x81; DEFAULT_MAX_DEPTH + 1];
        wire.push(0xf6);
        assert!(
            decode(&wire, &limits())
                .unwrap_err()
                .to_string()
                .contains("depth")
        );
    }

    #[test]
    fn declared_lengths_are_checked_before_traversal() {
        for first in [0x5a, 0x7a, 0x9a, 0xba] {
            let over = if first == 0x9a || first == 0xba {
                DEFAULT_MAX_CONTAINER_LENGTH + 1
            } else {
                DEFAULT_MAX_BYTE_LENGTH + 1
            };
            let mut wire = vec![first];
            wire.extend_from_slice(&(over as u32).to_be_bytes());
            assert!(
                decode(&wire, &limits())
                    .unwrap_err()
                    .to_string()
                    .contains("limit"),
                "{first:#x}"
            );
        }
    }

    #[test]
    fn a_small_input_cannot_force_a_large_allocation() {
        // Declares the maximum array length but supplies nothing.
        let mut wire = vec![0x9a];
        wire.extend_from_slice(&(DEFAULT_MAX_CONTAINER_LENGTH as u32).to_be_bytes());
        assert!(matches!(decode(&wire, &limits()), Err(Error::Cbor(_))));
    }

    #[test]
    fn stricter_limits_apply() {
        let tight = Limits {
            max_container_length: 2,
            max_byte_length: 2,
            ..Limits::default()
        };
        assert!(decode(&from_hex("83010203"), &tight).is_err());
        // The whole input (3 bytes) exceeds a 2-byte limit, as in Pi's own test.
        let two_bytes = Limits {
            max_byte_length: 2,
            ..Limits::default()
        };
        assert!(
            decode(&from_hex("626162"), &two_bytes)
                .unwrap_err()
                .to_string()
                .contains("limit")
        );
        assert!(encode(&json!([1, 2, 3]), &tight).is_err());
        assert!(encode(&json!("abc"), &tight).is_err());
        assert!(
            encode(
                &json!("ab"),
                &Limits {
                    max_byte_length: 3,
                    ..Limits::default()
                }
            )
            .is_ok()
        );
    }
}
