use bytes::{BufMut, Bytes, BytesMut};
use futures::stream::Stream;
use serde::ser::{self, Serialize, SerializeSeq};

pub fn to_stream<T: Serialize + ?Sized>(value: &T) -> impl Stream<Item = Bytes> + use<T> {
    let mut ser = Serializer::new();
    value.serialize(&mut ser).unwrap();
    ser.into_stream()
}

const CHUNK_SIZE: usize = 8192;

struct Serializer {
    completed_chunks: Vec<Bytes>,
    current_buffer: BytesMut,
}

impl Default for Serializer {
    fn default() -> Self {
        Self::new()
    }
}

impl Serializer {
    fn new() -> Self {
        Serializer {
            completed_chunks: Vec::new(),
            current_buffer: BytesMut::with_capacity(CHUNK_SIZE),
        }
    }

    fn write_bytes(&mut self, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            let remaining = CHUNK_SIZE - self.current_buffer.len();
            let write_len = remaining.min(bytes.len());
            self.current_buffer.put(&bytes[..write_len]);
            bytes = &bytes[write_len..];
            if self.current_buffer.len() == CHUNK_SIZE {
                let chunk = self.current_buffer.split().freeze();
                self.current_buffer.reserve(CHUNK_SIZE);
                self.completed_chunks.push(chunk);
            }
        }
    }

    fn write_string_value(&mut self, value: &str) {
        self.write_bytes(b"\"");
        let bytes = value.as_bytes();
        let mut start = 0;

        for (i, &byte) in bytes.iter().enumerate() {
            let escape = match byte {
                b'"' => Some("\\\""),
                b'\\' => Some("\\\\"),
                b'/' => Some("\\/"),
                b'\x08' => Some("\\b"),
                b'\x0c' => Some("\\f"),
                b'\n' => Some("\\n"),
                b'\r' => Some("\\r"),
                b'\t' => Some("\\t"),
                b @ 0..=0x1f => {
                    if start < i {
                        self.write_bytes(&bytes[start..i]);
                    }
                    static CONTROL: &[u8] = b"\\u0000\\u0001\\u0002\\u0003\\u0004\\u0005\\u0006\\u0007\\b\\t\\n\\u000b\\u000c\\r\\u000e\\u000f\\u0010\\u0011\\u0012\\u0013\\u0014\\u0015\\u0016\\u0017\\u0018\\u0019\\u001a\\u001b\\u001c\\u001d\\u001e\\u001f";
                    let ctrl_start = (b as usize) * 6;
                    self.write_bytes(&CONTROL[ctrl_start..ctrl_start + 6]);
                    start = i + 1;
                    None
                }
                _ => None,
            };

            if let Some(esc) = escape {
                if start < i {
                    self.write_bytes(&bytes[start..i]);
                }
                self.write_bytes(esc.as_bytes());
                start = i + 1;
            }
        }

        if start != bytes.len() {
            self.write_bytes(&bytes[start..]);
        }

        self.write_bytes(b"\"");
    }

    fn write(&mut self, value: impl ToString) {
        self.write_bytes(value.to_string().as_bytes());
    }

    fn into_stream(mut self) -> impl Stream<Item = Bytes> {
        if !self.current_buffer.is_empty() {
            self.completed_chunks.push(self.current_buffer.freeze());
        }
        futures::stream::iter(self.completed_chunks)
    }
}

#[derive(Debug)]
enum State {
    Empty,
    First,
    Rest,
}

enum Compound<'a> {
    Seq {
        ser: &'a mut Serializer,
        state: State,
    },
    Tuple {
        ser: &'a mut Serializer,
        state: State,
    },
    TupleStruct {
        ser: &'a mut Serializer,
        state: State,
    },
    Map {
        ser: &'a mut Serializer,
        state: State,
    },
    Struct {
        ser: &'a mut Serializer,
        state: State,
    },
    TupleVariant {
        ser: &'a mut Serializer,
        state: State,
    },
    StructVariant {
        ser: &'a mut Serializer,
        state: State,
    },
}

impl<'a> ser::SerializeSeq for Compound<'a> {
    type Ok = ();
    type Error = serde::de::value::Error;

    fn serialize_element<T: Serialize + ?Sized>(
        &mut self,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        let state = match self {
            Compound::Seq { state, .. } => state,
            _ => unreachable!(),
        };

        match state {
            State::Empty => {
                *state = State::First;
            }
            State::First => {
                *state = State::Rest;
            }
            State::Rest => {}
        }

        let (ser, state) = match self {
            Compound::Seq { ser, state } => (ser, state),
            _ => unreachable!(),
        };

        if !matches!(state, State::First) {
            ser.write_bytes(b",");
        }
        value.serialize(&mut **ser)?;
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        let Compound::Seq { ser, .. } = self else {
            unreachable!()
        };
        ser.write_bytes(b"]");
        Ok(())
    }
}

impl<'a> ser::SerializeTuple for Compound<'a> {
    type Ok = ();
    type Error = serde::de::value::Error;

    fn serialize_element<T: Serialize + ?Sized>(
        &mut self,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        let state = match self {
            Compound::Tuple { state, .. } => state,
            _ => unreachable!(),
        };

        match state {
            State::Empty => {
                *state = State::First;
            }
            State::First => {
                *state = State::Rest;
            }
            State::Rest => {}
        }

        let (ser, state) = match self {
            Compound::Tuple { ser, state } => (ser, state),
            _ => unreachable!(),
        };

        if !matches!(state, State::First) {
            ser.write_bytes(b",");
        }
        value.serialize(&mut **ser)?;
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        let Compound::Tuple { ser, .. } = self else {
            unreachable!()
        };
        ser.write_bytes(b"]");
        Ok(())
    }
}

impl<'a> ser::SerializeTupleStruct for Compound<'a> {
    type Ok = ();
    type Error = serde::de::value::Error;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        let state = match self {
            Compound::TupleStruct { state, .. } => state,
            _ => unreachable!(),
        };

        match state {
            State::Empty => {
                *state = State::First;
            }
            State::First => {
                *state = State::Rest;
            }
            State::Rest => {}
        }

        let (ser, state) = match self {
            Compound::TupleStruct { ser, state } => (ser, state),
            _ => unreachable!(),
        };

        if !matches!(state, State::First) {
            ser.write_bytes(b",");
        }
        value.serialize(&mut **ser)?;
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        let Compound::TupleStruct { ser, .. } = self else {
            unreachable!()
        };
        ser.write_bytes(b"]");
        Ok(())
    }
}

impl<'a> ser::SerializeTupleVariant for Compound<'a> {
    type Ok = ();
    type Error = serde::de::value::Error;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        let state = match self {
            Compound::TupleVariant { state, .. } => state,
            _ => unreachable!(),
        };

        match state {
            State::Empty => {
                *state = State::First;
            }
            State::First => {
                *state = State::Rest;
            }
            State::Rest => {}
        }

        let (ser, state) = match self {
            Compound::TupleVariant { ser, state } => (ser, state),
            _ => unreachable!(),
        };

        if !matches!(state, State::First) {
            ser.write_bytes(b",");
        }
        value.serialize(&mut **ser)?;
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        let Compound::TupleVariant { ser, .. } = self else {
            unreachable!()
        };
        ser.write_bytes(b"]");
        ser.write_bytes(b"}");
        Ok(())
    }
}

impl<'a> ser::SerializeMap for Compound<'a> {
    type Ok = ();
    type Error = serde::de::value::Error;

    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<Self::Ok, Self::Error> {
        let state = match self {
            Compound::Map { state, .. } => state,
            _ => unreachable!(),
        };

        match state {
            State::Empty => {
                *state = State::First;
            }
            State::First => {
                *state = State::Rest;
            }
            State::Rest => {}
        }

        let (ser, state) = match self {
            Compound::Map { ser, state } => (ser, state),
            _ => unreachable!(),
        };

        if !matches!(state, State::First) {
            ser.write_bytes(b",");
        }
        key.serialize(MapKeySerializer { ser })?;
        ser.write_bytes(b":");
        Ok(())
    }

    fn serialize_value<T: Serialize + ?Sized>(
        &mut self,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        let (ser,) = match self {
            Compound::Map { ser, .. } => (ser,),
            _ => unreachable!(),
        };

        value.serialize(&mut **ser)?;
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        let Compound::Map { ser, .. } = self else {
            unreachable!()
        };
        ser.write_bytes(b"}");
        Ok(())
    }
}

impl<'a> ser::SerializeStruct for Compound<'a> {
    type Ok = ();
    type Error = serde::de::value::Error;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        let state = match self {
            Compound::Struct { state, .. } => state,
            _ => unreachable!(),
        };

        match state {
            State::Empty => {
                *state = State::First;
            }
            State::First => {
                *state = State::Rest;
            }
            State::Rest => {}
        }

        let (ser, state) = match self {
            Compound::Struct { ser, state } => (ser, state),
            _ => unreachable!(),
        };

        if !matches!(state, State::First) {
            ser.write_bytes(b",");
        }
        ser.write_string_value(key);
        ser.write_bytes(b":");
        value.serialize(&mut **ser)?;
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        let Compound::Struct { ser, .. } = self else {
            unreachable!()
        };
        ser.write_bytes(b"}");
        Ok(())
    }
}

impl<'a> ser::SerializeStructVariant for Compound<'a> {
    type Ok = ();
    type Error = serde::de::value::Error;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        let state = match self {
            Compound::StructVariant { state, .. } => state,
            _ => unreachable!(),
        };

        match state {
            State::Empty => {
                *state = State::First;
            }
            State::First => {
                *state = State::Rest;
            }
            State::Rest => {}
        }

        let (ser, state) = match self {
            Compound::StructVariant { ser, state } => (ser, state),
            _ => unreachable!(),
        };

        if !matches!(state, State::First) {
            ser.write_bytes(b",");
        }
        ser.write_string_value(key);
        ser.write_bytes(b":");
        value.serialize(&mut **ser)?;
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        let Compound::StructVariant { ser, .. } = self else {
            unreachable!()
        };
        ser.write_bytes(b"}");
        ser.write_bytes(b"}");
        Ok(())
    }
}

enum Impossible {}

impl ser::SerializeSeq for Impossible {
    type Ok = ();
    type Error = serde::de::value::Error;

    fn serialize_element<T: Serialize + ?Sized>(
        &mut self,
        _value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        match *self {}
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        match self {}
    }
}

impl ser::SerializeTuple for Impossible {
    type Ok = ();
    type Error = serde::de::value::Error;

    fn serialize_element<T: Serialize + ?Sized>(
        &mut self,
        _value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        match *self {}
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        match self {}
    }
}

impl ser::SerializeTupleStruct for Impossible {
    type Ok = ();
    type Error = serde::de::value::Error;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        _value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        match *self {}
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        match self {}
    }
}

impl ser::SerializeTupleVariant for Impossible {
    type Ok = ();
    type Error = serde::de::value::Error;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        _value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        match *self {}
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        match self {}
    }
}

impl ser::SerializeMap for Impossible {
    type Ok = ();
    type Error = serde::de::value::Error;

    fn serialize_key<T: Serialize + ?Sized>(&mut self, _key: &T) -> Result<Self::Ok, Self::Error> {
        match *self {}
    }

    fn serialize_value<T: Serialize + ?Sized>(
        &mut self,
        _value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        match *self {}
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        match self {}
    }
}

impl ser::SerializeStruct for Impossible {
    type Ok = ();
    type Error = serde::de::value::Error;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        _key: &'static str,
        _value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        match *self {}
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        match self {}
    }
}

impl ser::SerializeStructVariant for Impossible {
    type Ok = ();
    type Error = serde::de::value::Error;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        _key: &'static str,
        _value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        match *self {}
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        match self {}
    }
}

struct MapKeySerializer<'a> {
    ser: &'a mut Serializer,
}

impl<'a> ser::Serializer for MapKeySerializer<'a> {
    type Ok = ();
    type Error = serde::de::value::Error;

    type SerializeSeq = Impossible;
    type SerializeTuple = Impossible;
    type SerializeTupleStruct = Impossible;
    type SerializeTupleVariant = Impossible;
    type SerializeMap = Impossible;
    type SerializeStruct = Impossible;
    type SerializeStructVariant = Impossible;

    fn serialize_bool(self, v: bool) -> Result<Self::Ok, Self::Error> {
        self.ser
            .write_string_value(if v { "true" } else { "false" });
        Ok(())
    }

    fn serialize_i8(self, v: i8) -> Result<Self::Ok, Self::Error> {
        self.ser.write_string_value(&v.to_string());
        Ok(())
    }

    fn serialize_i16(self, v: i16) -> Result<Self::Ok, Self::Error> {
        self.ser.write_string_value(&v.to_string());
        Ok(())
    }

    fn serialize_i32(self, v: i32) -> Result<Self::Ok, Self::Error> {
        self.ser.write_string_value(&v.to_string());
        Ok(())
    }

    fn serialize_i64(self, v: i64) -> Result<Self::Ok, Self::Error> {
        self.ser.write_string_value(&v.to_string());
        Ok(())
    }

    fn serialize_u8(self, v: u8) -> Result<Self::Ok, Self::Error> {
        self.ser.write_string_value(&v.to_string());
        Ok(())
    }

    fn serialize_u16(self, v: u16) -> Result<Self::Ok, Self::Error> {
        self.ser.write_string_value(&v.to_string());
        Ok(())
    }

    fn serialize_u32(self, v: u32) -> Result<Self::Ok, Self::Error> {
        self.ser.write_string_value(&v.to_string());
        Ok(())
    }

    fn serialize_u64(self, v: u64) -> Result<Self::Ok, Self::Error> {
        self.ser.write_string_value(&v.to_string());
        Ok(())
    }

    fn serialize_f32(self, v: f32) -> Result<Self::Ok, Self::Error> {
        self.ser.write_string_value(&v.to_string());
        Ok(())
    }

    fn serialize_f64(self, v: f64) -> Result<Self::Ok, Self::Error> {
        self.ser.write_string_value(&v.to_string());
        Ok(())
    }

    fn serialize_char(self, v: char) -> Result<Self::Ok, Self::Error> {
        self.ser.write_string_value(&v.to_string());
        Ok(())
    }

    fn serialize_str(self, v: &str) -> Result<Self::Ok, Self::Error> {
        self.ser.write_string_value(v);
        Ok(())
    }

    fn serialize_bytes(self, v: &[u8]) -> Result<Self::Ok, Self::Error> {
        self.ser.write_string_value(&String::from_utf8_lossy(v));
        Ok(())
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        self.ser.write_string_value("null");
        Ok(())
    }

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<Self::Ok, Self::Error> {
        value.serialize(self.ser)
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        self.ser.write_string_value("");
        Ok(())
    }

    fn serialize_unit_struct(self, name: &'static str) -> Result<Self::Ok, Self::Error> {
        self.ser.write_string_value(name);
        Ok(())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        self.ser.write_string_value(variant);
        Ok(())
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        value.serialize(self.ser)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        self.ser.write_string_value(variant);
        Ok(())
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Impossible, Self::Error> {
        unreachable!()
    }

    fn serialize_tuple(self, _len: usize) -> Result<Impossible, Self::Error> {
        unreachable!()
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Impossible, Self::Error> {
        unreachable!()
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Impossible, Self::Error> {
        unreachable!()
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Impossible, Self::Error> {
        unreachable!()
    }

    fn serialize_struct(self, _name: &'static str, _len: usize) -> Result<Impossible, Self::Error> {
        unreachable!()
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Impossible, Self::Error> {
        unreachable!()
    }
}

impl<'a> ser::Serializer for &'a mut Serializer {
    type Ok = ();
    type Error = serde::de::value::Error;

    type SerializeSeq = Compound<'a>;
    type SerializeTuple = Compound<'a>;
    type SerializeTupleStruct = Compound<'a>;
    type SerializeTupleVariant = Compound<'a>;
    type SerializeMap = Compound<'a>;
    type SerializeStruct = Compound<'a>;
    type SerializeStructVariant = Compound<'a>;

    fn serialize_bool(self, v: bool) -> Result<Self::Ok, Self::Error> {
        self.write_bytes(if v { b"true" } else { b"false" });
        Ok(())
    }

    fn serialize_i8(self, v: i8) -> Result<Self::Ok, Self::Error> {
        self.write(v);
        Ok(())
    }

    fn serialize_i16(self, v: i16) -> Result<Self::Ok, Self::Error> {
        self.write(v);
        Ok(())
    }

    fn serialize_i32(self, v: i32) -> Result<Self::Ok, Self::Error> {
        self.write(v);
        Ok(())
    }

    fn serialize_i64(self, v: i64) -> Result<Self::Ok, Self::Error> {
        self.write(v);
        Ok(())
    }

    fn serialize_u8(self, v: u8) -> Result<Self::Ok, Self::Error> {
        self.write(v);
        Ok(())
    }

    fn serialize_u16(self, v: u16) -> Result<Self::Ok, Self::Error> {
        self.write(v);
        Ok(())
    }

    fn serialize_u32(self, v: u32) -> Result<Self::Ok, Self::Error> {
        self.write(v);
        Ok(())
    }

    fn serialize_u64(self, v: u64) -> Result<Self::Ok, Self::Error> {
        self.write(v);
        Ok(())
    }

    fn serialize_f32(self, v: f32) -> Result<Self::Ok, Self::Error> {
        self.write(v);
        Ok(())
    }

    fn serialize_f64(self, v: f64) -> Result<Self::Ok, Self::Error> {
        self.write(v);
        Ok(())
    }

    fn serialize_char(self, v: char) -> Result<Self::Ok, Self::Error> {
        self.write_string_value(&v.to_string());
        Ok(())
    }

    fn serialize_str(self, v: &str) -> Result<Self::Ok, Self::Error> {
        self.write_string_value(v);
        Ok(())
    }

    fn serialize_bytes(self, v: &[u8]) -> Result<Self::Ok, Self::Error> {
        let mut seq = self.serialize_seq(Some(v.len()))?;
        for b in v {
            seq.serialize_element(b)?;
        }
        seq.end()?;
        Ok(())
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        self.write_bytes(b"null");
        Ok(())
    }

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        self.write_bytes(b"null");
        Ok(())
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
        self.write_bytes(b"null");
        Ok(())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        self.write_bytes(b"{");
        self.write_bytes(b"\"t\":");
        self.write_string_value(variant);
        self.write_bytes(b",\"v\":null");
        self.write_bytes(b"}");
        Ok(())
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        self.write_bytes(b"{");
        self.write_bytes(b"\"t\":");
        self.write_string_value(variant);
        self.write_bytes(b",\"v\":");
        value.serialize(&mut *self)?;
        self.write_bytes(b"}");
        Ok(())
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Compound<'a>, Self::Error> {
        self.write_bytes(b"[");
        Ok(Compound::Seq {
            ser: self,
            state: State::Empty,
        })
    }

    fn serialize_tuple(self, _len: usize) -> Result<Compound<'a>, Self::Error> {
        self.write_bytes(b"[");
        Ok(Compound::Tuple {
            ser: self,
            state: State::Empty,
        })
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Compound<'a>, Self::Error> {
        self.write_bytes(b"[");
        Ok(Compound::TupleStruct {
            ser: self,
            state: State::Empty,
        })
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Compound<'a>, Self::Error> {
        self.write_bytes(b"{");
        self.write_bytes(b"\"t\":");
        self.write_string_value(variant);
        self.write_bytes(b",\"v\":[");
        Ok(Compound::TupleVariant {
            ser: self,
            state: State::Empty,
        })
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Compound<'a>, Self::Error> {
        self.write_bytes(b"{");
        Ok(Compound::Map {
            ser: self,
            state: State::Empty,
        })
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Compound<'a>, Self::Error> {
        self.write_bytes(b"{");
        Ok(Compound::Struct {
            ser: self,
            state: State::Empty,
        })
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Compound<'a>, Self::Error> {
        self.write_bytes(b"{");
        self.write_bytes(b"\"t\":");
        self.write_string_value(variant);
        self.write_bytes(b",\"v\":{");
        Ok(Compound::StructVariant {
            ser: self,
            state: State::Empty,
        })
    }
}
