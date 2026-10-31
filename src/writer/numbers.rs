//! The numbers a display item carries into PDF operators: each must be finite and within
//! what PDF readers represent, or the item is not written.
use serde::ser::{self, Serialize};

/// The bound on the magnitude of a number written to PDF: ISO 32000-1 Annex C.2 Table C.1
/// limits integers to 2^31 - 1, and readers built on 32-bit numbers represent no larger value.
pub(super) const PDF_NUMBER_BOUND: f32 = 2_147_483_648.0;

/// Fields, by struct, whose numbers are not written as they are held: nested display lists
/// and text lines, whose items are checked as they are drawn; corner radii, which the writer
/// reduces to fit the box (CSS Backgrounds 3 §5.5); corner shapes, which place no coordinate;
/// a box's outline, which paints as an item of its own; and filter functions, which are
/// rasterized.
const UNWRITTEN_FIELDS: [(&str, &str); 8] = [
    ("FlowPaint", "items"),
    ("FlowPaint", "lines"),
    ("PlanePaint", "items"),
    ("BoxDecoration", "radii"),
    ("BoxDecoration", "corner_shapes"),
    ("BoxDecoration", "outline"),
    ("FilterPaint", "functions"),
    ("BackdropPaint", "functions"),
];
/// The variant holding a calculation, which resolves to a finite number at use: CSS Values 4
/// §10.9 censors its NaN and infinities.
const CALCULATION_VARIANT: &str = "Calc";

/// The first number `value` holds that the writer would write and that is not finite or not
/// within [`PDF_NUMBER_BOUND`].
pub(super) fn unwritable_number(value: &impl Serialize) -> Option<f64> {
    match value.serialize(Scan) {
        Ok(()) => None,
        Err(Unwritable(number)) => Some(number),
    }
}

#[derive(Debug)]
struct Unwritable(f64);
impl std::fmt::Display for Unwritable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "unwritable number {}", self.0)
    }
}
impl std::error::Error for Unwritable {}
impl ser::Error for Unwritable {
    fn custom<T: std::fmt::Display>(_: T) -> Self {
        Self(f64::NAN)
    }
}

/// A serializer that only visits numbers, stopping at the first unwritable one.
struct Scan;

fn number(value: f64) -> Result<(), Unwritable> {
    if value.is_finite() && value.abs() < f64::from(PDF_NUMBER_BOUND) {
        Ok(())
    } else {
        Err(Unwritable(value))
    }
}

macro_rules! accept {
    ($($method:ident($type:ty)),* $(,)?) => {
        $(fn $method(self, _: $type) -> Result<(), Unwritable> { Ok(()) })*
    };
}

impl ser::Serializer for Scan {
    type Ok = ();
    type Error = Unwritable;
    type SerializeSeq = Self;
    type SerializeTuple = Self;
    type SerializeTupleStruct = Self;
    type SerializeTupleVariant = Self;
    type SerializeMap = Self;
    type SerializeStruct = Fields;
    type SerializeStructVariant = Fields;

    accept!(
        serialize_bool(bool),
        serialize_i8(i8),
        serialize_i16(i16),
        serialize_i32(i32),
        serialize_i64(i64),
        serialize_u8(u8),
        serialize_u16(u16),
        serialize_u32(u32),
        serialize_u64(u64),
        serialize_char(char),
        serialize_str(&str),
        serialize_bytes(&[u8]),
        serialize_unit_struct(&'static str),
    );
    fn serialize_f32(self, value: f32) -> Result<(), Unwritable> {
        number(f64::from(value))
    }
    fn serialize_f64(self, value: f64) -> Result<(), Unwritable> {
        number(value)
    }
    fn serialize_none(self) -> Result<(), Unwritable> {
        Ok(())
    }
    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<(), Unwritable> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> Result<(), Unwritable> {
        Ok(())
    }
    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
    ) -> Result<(), Unwritable> {
        Ok(())
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        value: &T,
    ) -> Result<(), Unwritable> {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<(), Unwritable> {
        if variant == CALCULATION_VARIANT {
            return Ok(());
        }
        value.serialize(self)
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self, Unwritable> {
        Ok(self)
    }
    fn serialize_tuple(self, _: usize) -> Result<Self, Unwritable> {
        Ok(self)
    }
    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Self, Unwritable> {
        Ok(self)
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self, Unwritable> {
        Ok(self)
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self, Unwritable> {
        Ok(self)
    }
    fn serialize_struct(self, name: &'static str, _: usize) -> Result<Fields, Unwritable> {
        Ok(Fields(name))
    }
    fn serialize_struct_variant(
        self,
        name: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Fields, Unwritable> {
        Ok(Fields(name))
    }
}

macro_rules! elements {
    ($($trait:ident::$method:ident),* $(,)?) => {
        $(impl ser::$trait for Scan {
            type Ok = ();
            type Error = Unwritable;
            fn $method<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Unwritable> {
                value.serialize(Scan)
            }
            fn end(self) -> Result<(), Unwritable> {
                Ok(())
            }
        })*
    };
}
elements!(
    SerializeSeq::serialize_element,
    SerializeTuple::serialize_element,
    SerializeTupleStruct::serialize_field,
    SerializeTupleVariant::serialize_field,
);

/// The fields of the struct it names, scanned unless [`UNWRITTEN_FIELDS`] holds them.
struct Fields(&'static str);

macro_rules! fields {
    ($($trait:ident),* $(,)?) => {
        $(impl ser::$trait for Fields {
            type Ok = ();
            type Error = Unwritable;
            fn serialize_field<T: Serialize + ?Sized>(
                &mut self,
                key: &'static str,
                value: &T,
            ) -> Result<(), Unwritable> {
                if UNWRITTEN_FIELDS.contains(&(self.0, key)) {
                    return Ok(());
                }
                value.serialize(Scan)
            }
            fn end(self) -> Result<(), Unwritable> {
                Ok(())
            }
        })*
    };
}
fields!(SerializeStruct, SerializeStructVariant);

impl ser::SerializeMap for Scan {
    type Ok = ();
    type Error = Unwritable;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), Unwritable> {
        key.serialize(Scan)
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Unwritable> {
        value.serialize(Scan)
    }
    fn end(self) -> Result<(), Unwritable> {
        Ok(())
    }
}
