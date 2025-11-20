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
        Type::S8 => Val::S8(json_val.as_i64()? as i8),
        Type::U8 => Val::U8(json_val.as_u64()? as u8),
        Type::S16 => Val::S16(json_val.as_i64()? as i16),
        Type::U16 => Val::U16(json_val.as_u64()? as u16),
        Type::S32 => Val::S32(json_val.as_i64()? as i32),
        Type::U32 => Val::U32(json_val.as_u64()? as u32),
        Type::S64 => Val::S64(json_val.as_i64()?),
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
        Val::Variant(variant_name, val) => {
            let mut array = sonic_rs::Array::new();
            array.push(sonic_rs::Value::from_str(&variant_name).ok()?);
            if let Some(boxed_val) = val {
                array.push(convert_wit_val_to_json_value(*boxed_val)?);
            }
            Some(array.into_value())
        }
        Val::Enum(name) => Some(sonic_rs::Value::from_str(&name).ok()?),
        Val::Option(val) => match val {
            None => Some(sonic_rs::Value::new_null()),
            Some(boxed_val) => convert_wit_val_to_json_value(*boxed_val),
        },
        Val::Result(result) => {
            let mut object = sonic_rs::Object::new();
            match result {
                Ok(Some(boxed_val)) => {
                    object.insert("ok", convert_wit_val_to_json_value(*boxed_val)?);
                }
                Ok(None) => {
                    object.insert("ok", sonic_rs::Value::new_null());
                }
                Err(Some(boxed_val)) => {
                    object.insert("err", convert_wit_val_to_json_value(*boxed_val)?);
                }
                Err(None) => {
                    object.insert("err", sonic_rs::Value::new_null());
                }
            }
            Some(object.into_value())
        }
        Val::Flags(items) => {
            let mut array = sonic_rs::Array::with_capacity(items.len());
            for flag in items {
                array.push(sonic_rs::Value::from_str(&flag).ok()?);
            }
            Some(array.into_value())
        }
        Val::Resource(_) | Val::Future(_) | Val::Stream(_) | Val::ErrorContext(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sonic_rs::JsonContainerTrait;
    use wasmtime::{Engine, Store};

    fn create_test_component() -> (Engine, Component) {
        let engine = Engine::default();

        let wat = r#"
            (component
                (core module $m
                    (func (export "test-bool") (param i32) (result i32)
                        local.get 0
                    )
                    (func (export "test-u32") (param i32) (result i32)
                        local.get 0
                    )
                    (func (export "test-s32") (param i32) (result i32)
                        local.get 0
                    )
                    (func (export "test-u64") (param i64) (result i64)
                        local.get 0
                    )
                    (func (export "test-float32") (param f32) (result f32)
                        local.get 0
                    )
                    (func (export "test-float64") (param f64) (result f64)
                        local.get 0
                    )
                    (memory (export "memory") 1)
                )
                (core instance $i (instantiate $m))

                (func (export "test-bool") (param "x" bool) (result bool)
                    (canon lift (core func $i "test-bool") (memory $i "memory"))
                )
                (func (export "test-u32") (param "x" u32) (result u32)
                    (canon lift (core func $i "test-u32") (memory $i "memory"))
                )
                (func (export "test-s32") (param "x" s32) (result s32)
                    (canon lift (core func $i "test-s32") (memory $i "memory"))
                )
                (func (export "test-u64") (param "x" u64) (result u64)
                    (canon lift (core func $i "test-u64") (memory $i "memory"))
                )
                (func (export "test-float32") (param "x" float32) (result float32)
                    (canon lift (core func $i "test-float32") (memory $i "memory"))
                )
                (func (export "test-float64") (param "x" float64) (result float64)
                    (canon lift (core func $i "test-float64") (memory $i "memory"))
                )
            )
        "#;

        let component = Component::new(&engine, wat).unwrap();
        (engine, component)
    }

    fn get_function_param_types(
        engine: &Engine,
        component: &Component,
        func_name: &str,
    ) -> Box<[(String, Type)]> {
        let linker = Linker::new(engine);
        let mut store = Store::new(engine, ());
        let instance = linker.instantiate(&mut store, component).unwrap();
        let func = instance.get_func(&mut store, func_name).unwrap();
        func.params(&store)
    }

    #[test]
    fn test_convert_wit_vals_to_json_primitives() {
        let json = convert_wit_vals_to_json(vec![Val::Bool(true)]).unwrap();
        assert_eq!(json, b"true");

        let json = convert_wit_vals_to_json(vec![Val::U32(42)]).unwrap();
        assert_eq!(json, b"42");

        let json = convert_wit_vals_to_json(vec![Val::S32(-10)]).unwrap();
        assert_eq!(json, b"-10");

        let json = convert_wit_vals_to_json(vec![Val::String("hello".to_string())]).unwrap();
        assert_eq!(json, br#""hello""#);
    }

    #[test]
    fn test_convert_wit_vals_to_json_multiple_values() {
        let json = convert_wit_vals_to_json(vec![Val::U32(1), Val::U32(2)]).unwrap();
        assert_eq!(json, b"[1,2]");
    }

    #[test]
    fn test_convert_wit_vals_to_json_list() {
        let list = Val::List(vec![Val::U32(1), Val::U32(2), Val::U32(3)]);
        let json = convert_wit_vals_to_json(vec![list]).unwrap();
        assert_eq!(json, b"[1,2,3]");
    }

    #[test]
    fn test_convert_wit_vals_to_json_record() {
        let record = Val::Record(vec![
            ("x".to_string(), Val::U32(10)),
            ("y".to_string(), Val::U32(20)),
        ]);
        let json = convert_wit_vals_to_json(vec![record]).unwrap();
        let parsed: sonic_rs::Value = sonic_rs::from_slice(&json).unwrap();
        assert_eq!(parsed["x"].as_u64(), Some(10));
        assert_eq!(parsed["y"].as_u64(), Some(20));
    }

    #[test]
    fn test_convert_wit_vals_to_json_tuple() {
        let tuple = Val::Tuple(vec![Val::U32(42), Val::String("test".to_string())]);
        let json = convert_wit_vals_to_json(vec![tuple]).unwrap();
        let parsed: sonic_rs::Value = sonic_rs::from_slice(&json).unwrap();
        assert_eq!(parsed[0].as_u64(), Some(42));
        assert_eq!(parsed[1].as_str(), Some("test"));
    }

    #[test]
    fn test_convert_wit_vals_to_json_option() {
        let some_val = Val::Option(Some(Box::new(Val::U32(42))));
        let json = convert_wit_vals_to_json(vec![some_val]).unwrap();
        assert_eq!(json, b"42");

        let none_val = Val::Option(None);
        let json = convert_wit_vals_to_json(vec![none_val]).unwrap();
        assert_eq!(json, b"null");
    }

    #[test]
    fn test_convert_wit_vals_to_json_result() {
        let ok_val = Val::Result(Ok(Some(Box::new(Val::U32(42)))));
        let json = convert_wit_vals_to_json(vec![ok_val]).unwrap();
        let parsed: sonic_rs::Value = sonic_rs::from_slice(&json).unwrap();
        assert_eq!(parsed["ok"].as_u64(), Some(42));

        let err_val = Val::Result(Err(Some(Box::new(Val::String("error".to_string())))));
        let json = convert_wit_vals_to_json(vec![err_val]).unwrap();
        let parsed: sonic_rs::Value = sonic_rs::from_slice(&json).unwrap();
        assert_eq!(parsed["err"].as_str(), Some("error"));
    }

    #[test]
    fn test_convert_wit_vals_to_json_variant() {
        let variant = Val::Variant("ok".to_string(), Some(Box::new(Val::U32(42))));
        let json = convert_wit_vals_to_json(vec![variant]).unwrap();
        let parsed: sonic_rs::Value = sonic_rs::from_slice(&json).unwrap();
        assert_eq!(parsed[0].as_str(), Some("ok"));
        assert_eq!(parsed[1].as_u64(), Some(42));

        let variant_no_data = Val::Variant("error".to_string(), None);
        let json = convert_wit_vals_to_json(vec![variant_no_data]).unwrap();
        let parsed: sonic_rs::Value = sonic_rs::from_slice(&json).unwrap();
        assert_eq!(parsed[0].as_str(), Some("error"));
        assert_eq!(parsed.as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_convert_wit_vals_to_json_enum() {
        let enum_val = Val::Enum("red".to_string());
        let json = convert_wit_vals_to_json(vec![enum_val]).unwrap();
        assert_eq!(json, br#""red""#);
    }

    #[test]
    fn test_convert_wit_vals_to_json_flags() {
        let flags = Val::Flags(vec!["read".to_string(), "write".to_string()]);
        let json = convert_wit_vals_to_json(vec![flags]).unwrap();
        let parsed: sonic_rs::Value = sonic_rs::from_slice(&json).unwrap();
        let array = parsed.as_array().unwrap();
        let has_read = array.iter().any(|v| v.as_str() == Some("read"));
        let has_write = array.iter().any(|v| v.as_str() == Some("write"));
        assert!(has_read);
        assert!(has_write);
    }

    #[test]
    fn test_convert_json_to_vals_bool() {
        let (engine, component) = create_test_component();
        let types = get_function_param_types(&engine, &component, "test-bool");
        let json = br#"{"x": true}"#;
        let vals = convert_json_to_vals(types, json).unwrap();
        assert!(matches!(vals[0], Val::Bool(true)));
    }

    #[test]
    fn test_convert_json_to_vals_u32() {
        let (engine, component) = create_test_component();
        let types = get_function_param_types(&engine, &component, "test-u32");
        let json = br#"{"x": 42}"#;
        let vals = convert_json_to_vals(types, json).unwrap();
        assert!(matches!(vals[0], Val::U32(42)));
    }

    #[test]
    fn test_convert_json_to_vals_s32() {
        let (engine, component) = create_test_component();
        let types = get_function_param_types(&engine, &component, "test-s32");
        let json = br#"{"x": -10}"#;
        let vals = convert_json_to_vals(types, json).unwrap();
        assert!(matches!(vals[0], Val::S32(-10)));
    }

    #[test]
    fn test_convert_json_to_vals_u64() {
        let (engine, component) = create_test_component();
        let types = get_function_param_types(&engine, &component, "test-u64");
        let json = br#"{"x": 1000000}"#;
        let vals = convert_json_to_vals(types, json).unwrap();
        assert!(matches!(vals[0], Val::U64(1000000)));
    }

    #[test]
    fn test_roundtrip_u32() {
        let (engine, component) = create_test_component();
        let types = get_function_param_types(&engine, &component, "test-u32");

        let original_json = br#"{"x": 42}"#;
        let vals = convert_json_to_vals(types.clone(), original_json).unwrap();
        let json_bytes = convert_wit_vals_to_json(vals).unwrap();
        assert_eq!(json_bytes, b"42");

        let vals_again = convert_json_to_vals(
            types,
            format!(r#"{{"x": {}}}"#, String::from_utf8(json_bytes).unwrap()).as_bytes(),
        )
        .unwrap();
        assert!(matches!(vals_again[0], Val::U32(42)));
    }

    #[test]
    fn test_roundtrip_bool() {
        let (engine, component) = create_test_component();
        let types = get_function_param_types(&engine, &component, "test-bool");

        let original_json = br#"{"x": true}"#;
        let vals = convert_json_to_vals(types.clone(), original_json).unwrap();
        let json_bytes = convert_wit_vals_to_json(vals).unwrap();
        assert_eq!(json_bytes, b"true");

        let vals_again = convert_json_to_vals(
            types,
            format!(r#"{{"x": {}}}"#, String::from_utf8(json_bytes).unwrap()).as_bytes(),
        )
        .unwrap();
        assert!(matches!(vals_again[0], Val::Bool(true)));
    }
}
