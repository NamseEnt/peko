use std::str::FromStr;

use sonic_rs::JsonValueTrait;
use wasmtime::component::*;

// TODO: Error Handling
pub fn convert_json_to_vals(types: Box<[(String, Type)]>, json_bytes: &[u8]) -> Option<Vec<Val>> {
    let mut json: sonic_rs::Object = sonic_rs::from_slice(json_bytes).ok()?;

    let mut vals = Vec::with_capacity(types.len());

    for (name, ty) in types {
        let json_val = json.remove(&name)?;
        vals.push(convert_json_value_to_wit_val(json_val, &ty)?);
    }

    Some(vals)
}

// TODO: Error Handling
pub fn convert_wit_vals_to_json(vals: Vec<Val>) -> Option<Vec<u8>> {
    let mut array = sonic_rs::Array::with_capacity(vals.len());
    for val in vals {
        array.push(convert_wit_val_to_json_value(val));
    }
    let json_value = if array.len() == 1 {
        array.swap_remove(0)
    } else {
        array.into_value()
    };

    Some(sonic_rs::to_vec(&json_value).unwrap())
}

fn convert_json_value_to_wit_val(json_val: sonic_rs::Value, ty: &Type) -> Option<Val> {
    Some(match ty {
        Type::Bool => Val::Bool(json_val.as_bool()?),
        Type::S8 => Val::S8(json_val.as_u64()? as i8),
        Type::U8 => Val::U8(json_val.as_u64()? as u8),
        Type::S16 => Val::S16(json_val.as_u64()? as i16),
        Type::U16 => Val::U16(json_val.as_u64()? as u16),
        Type::S32 => Val::S32(json_val.as_u64()? as i32),
        Type::U32 => Val::U32(json_val.as_u64()? as u32),
        Type::S64 => Val::S64(json_val.as_u64()? as i64),
        Type::U64 => Val::U64(json_val.as_u64()?),
        Type::Float32 => Val::Float32(json_val.as_f64()? as f32),
        Type::Float64 => Val::Float64(json_val.as_f64()?),
        Type::Char => Val::Char(json_val.as_str().and_then(|str| str.chars().next())?),
        Type::String => Val::String(json_val.as_str()?.to_string()),
        Type::List(list) => {
            let array = json_val.into_array()?;
            let item_type = list.ty();

            let mut list = Vec::with_capacity(array.len());

            for val in array {
                let item = convert_json_value_to_wit_val(val, &item_type)?;
                list.push(item);
            }

            Val::List(list)
        }
        Type::Record(record) => {
            let mut object = json_val.into_object()?;
            let mut fields = Vec::with_capacity(record.fields().len());

            for field in record.fields() {
                let val = convert_json_value_to_wit_val(object.remove(&field.name)?, &field.ty)?;
                fields.push((field.name.to_string(), val));
            }

            Val::Record(fields)
        }
        Type::Tuple(tuple) => {
            let array = json_val.into_array()?;
            let mut items = Vec::with_capacity(tuple.types().len());
            if array.len() != tuple.types().len() {
                return None;
            }

            for (val, ty) in array.into_iter().zip(tuple.types()) {
                let item = convert_json_value_to_wit_val(val, &ty)?;
                items.push(item);
            }

            Val::Tuple(items)
        }
        Type::Variant(variant) => {
            let mut array = json_val.into_array()?;
            if array.is_empty() {
                return None;
            }
            let variant_name = array.first()?.as_str()?.to_string();

            let case = variant.cases().find(|case| case.name == variant_name)?;

            let Some(ty) = case.ty else {
                if array.len() != 1 {
                    return None;
                }
                return Some(Val::Variant(variant_name.to_string(), None));
            };

            if array.len() != 2 {
                return None;
            }

            let variant_json_data = array.swap_remove(1);

            let variant_val = convert_json_value_to_wit_val(variant_json_data, &ty)?;

            Val::Variant(variant_name.to_string(), Some(Box::new(variant_val)))
        }
        Type::Enum(eenum) => {
            let string = json_val.as_str()?;
            if !eenum.names().any(|name| name == string) {
                return None;
            }
            Val::Enum(string.to_string())
        }
        Type::Option(option_type) => {
            let ty = option_type.ty();
            if json_val.is_null() {
                return Some(Val::Option(None));
            }
            let val = convert_json_value_to_wit_val(json_val, &ty)?;
            Val::Option(Some(Box::new(val)))
        }
        Type::Result(result_type) => {
            let mut object = json_val.into_object()?;
            if let Some(val) = object.remove(&"ok") {
                match result_type.ok() {
                    Some(ok_ty) => {
                        let val = convert_json_value_to_wit_val(val, &ok_ty)?;
                        return Some(Val::Result(Ok(Some(Box::new(val)))));
                    }
                    None => return Some(Val::Result(Ok(None))),
                };
            }
            if let Some(val) = object.remove(&"err") {
                match result_type.err() {
                    Some(err_ty) => {
                        let val = convert_json_value_to_wit_val(val, &err_ty)?;
                        return Some(Val::Result(Err(Some(Box::new(val)))));
                    }
                    None => return Some(Val::Result(Err(None))),
                };
            }
            return None;
        }
        Type::Flags(flags) => {
            let array = json_val.into_array()?;
            let mut vec = Vec::with_capacity(array.len());
            for flag in array {
                let flag = flag.as_str()?;
                if !flags.names().any(|name| name == flag) {
                    return None;
                }
                vec.push(flag.to_string());
            }
            Val::Flags(vec)
        }
        Type::Own(_) | Type::Borrow(_) | Type::Future(_) | Type::Stream(_) | Type::ErrorContext => {
            // Not Supported
            return None;
        }
    })
}

fn convert_wit_val_to_json_value(val: Val) -> Option<sonic_rs::Value> {
    match val {
        Val::Bool(val) => Some(sonic_rs::Value::new_bool(val)),
        Val::S8(val) => Some(sonic_rs::Value::new_i64(val as i64)),
        Val::U8(val) => Some(sonic_rs::Value::new_u64(val as u64)),
        Val::S16(val) => Some(sonic_rs::Value::new_i64(val as i64)),
        Val::U16(val) => Some(sonic_rs::Value::new_u64(val as u64)),
        Val::S32(val) => Some(sonic_rs::Value::new_i64(val as i64)),
        Val::U32(val) => Some(sonic_rs::Value::new_u64(val as u64)),
        Val::S64(val) => Some(sonic_rs::Value::new_i64(val)),
        Val::U64(val) => Some(sonic_rs::Value::new_u64(val)),
        Val::Float32(val) => Some(sonic_rs::Value::new_f64(val as f64))?,
        Val::Float64(val) => Some(sonic_rs::Value::new_f64(val))?,
        Val::Char(val) => Some(sonic_rs::Value::from_str(&val.to_string()).ok()?),
        Val::String(str) => Some(sonic_rs::Value::from_str(&str.to_string()).ok()?),
        Val::List(vals) => {
            let mut array = sonic_rs::Array::with_capacity(vals.len());
            for val in vals {
                array.push(convert_wit_val_to_json_value(val)?);
            }
            Some(array.into_value())
        }
        Val::Record(items) => {
            let mut object = sonic_rs::Object::with_capacity(items.len());
            for (name, val) in items {
                object.insert(&name, convert_wit_val_to_json_value(val)?);
            }
            Some(object.into_value())
        }
        Val::Tuple(vals) => {
            let mut array = sonic_rs::Array::with_capacity(vals.len());
            for val in vals {
                array.push(convert_wit_val_to_json_value(val)?);
            }
            Some(array.into_value())
        }
        Val::Variant(_, val) => todo!(),
        Val::Enum(_) => todo!(),
        Val::Option(val) => todo!(),
        Val::Result(val) => todo!(),
        Val::Flags(items) => todo!(),
        Val::Resource(_) | Val::Future(_) | Val::Stream(_) | Val::ErrorContext(_) => None,
    }
}
