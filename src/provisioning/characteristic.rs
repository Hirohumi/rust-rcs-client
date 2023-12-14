// Copyright 2023 宋昊文
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

extern crate quick_xml;

use std::{
    ffi::CString,
    io::Write,
    str::{self, FromStr},
};

use quick_xml::{
    events::{BytesEnd, BytesStart, Event},
    Reader, Writer,
};

pub struct Parameter {
    pub name: String,
    pub value: String,
}

impl Clone for Parameter {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            value: self.value.clone(),
        }
    }
}

pub fn read_parm<R>(
    xml_reader: &mut Reader<R>,
    buf: &mut Vec<u8>,
    e: &BytesStart,
    level: i32,
) -> Option<Parameter>
where
    R: std::io::BufRead,
{
    let mut name: Option<String> = None;
    let mut value: Option<String> = None;

    for attribute in e.attributes() {
        if let Ok(attribute) = attribute {
            if attribute.key.as_ref() == b"name" {
                if let Ok(attribute_value) = attribute.unescape_value() {
                    name.replace(attribute_value.into_owned());
                }
            } else if attribute.key.as_ref() == b"value" {
                if let Ok(attribute_value) = attribute.unescape_value() {
                    value.replace(attribute_value.into_owned());
                }
            }
        }
    }

    let mut level = level;

    if level > 0 {
        loop {
            match xml_reader.read_event_into(buf) {
                Ok(Event::Start(_)) => {
                    level += 1;
                    if level == 0 {
                        break;
                    }
                }

                Ok(Event::End(_)) => {
                    level -= 1;
                    if level == 0 {
                        break;
                    }
                }

                Ok(Event::Eof) | Err(_) => {
                    break;
                }

                _ => {}
            }
        }
    }

    if let (Some(name), Some(value)) = (name, value) {
        return Some(Parameter { name, value });
    }

    None
}

pub fn write_parm<W>(xml_writer: &mut Writer<W>, parameter: &Parameter) -> quick_xml::Result<()>
where
    W: Write,
{
    let mut elem = BytesStart::new("parm");

    elem.push_attribute(("name", parameter.name.as_str()));
    elem.push_attribute(("value", parameter.value.as_str()));

    xml_writer.write_event(Event::Empty(elem))?;

    Ok(())
}

pub struct Characteristic {
    pub characteristic_type: String,
    pub characteristic_parameters: Vec<Parameter>,
    pub child_characteristics: Vec<Characteristic>,
}

impl Characteristic {
    pub fn get_child_characteristic(&self, name: &str) -> Option<&Characteristic> {
        for child in &self.child_characteristics {
            if child.characteristic_type == name {
                return Some(&child);
            }
        }

        None
    }

    pub fn get_parameter(&self, name: &str) -> Option<&str> {
        for child in &self.characteristic_parameters {
            if child.name == name {
                return Some(&child.value);
            }
        }

        None
    }

    pub fn get_runtime_characteristics(&self) -> RuntimeCharacteristics {
        RuntimeCharacteristics {
            root: self.child_characteristics.iter(),
        }
    }

    pub fn to_string(&self) -> Result<String, ()> {
        let mut xml_writer = Writer::new(Vec::new());
        if let Ok(_) = write_characteristic(&mut xml_writer, self) {
            let v = xml_writer.into_inner();
            Ok(String::from_utf8_lossy(&v).to_string())
        } else {
            Err(())
        }
    }

    pub fn to_c_string(&self) -> Result<CString, ()> {
        let mut xml_writer = Writer::new(Vec::new());
        if let Ok(_) = write_characteristic(&mut xml_writer, self) {
            let v = xml_writer.into_inner();
            if let Ok(s) = str::from_utf8(&v) {
                if let Ok(c) = CString::new(s) {
                    return Ok(c);
                }
            }
        }
        Err(())
    }
}

impl Clone for Characteristic {
    fn clone(&self) -> Self {
        Self {
            characteristic_type: self.characteristic_type.clone(),
            characteristic_parameters: self.characteristic_parameters.clone(),
            child_characteristics: self.child_characteristics.clone(),
        }
    }
}

impl FromStr for Characteristic {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut xml_reader = Reader::from_str(s);
        let mut buf = Vec::new();

        let handle_element = |xml_reader: &mut Reader<&[u8]>,
                              e: &BytesStart,
                              level: i32|
         -> Option<Characteristic> {
            if e.name().as_ref().eq_ignore_ascii_case(b"characteristic") {
                let mut buf = Vec::new();
                return read_characteristic(xml_reader, &mut buf, e, level);
            }
            None
        };

        loop {
            match xml_reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    if let Some(characteristic) = handle_element(&mut xml_reader, e, 1) {
                        return Ok(characteristic);
                    }
                }

                Ok(Event::Empty(ref e)) => {
                    if let Some(characteristic) = handle_element(&mut xml_reader, e, 0) {
                        return Ok(characteristic);
                    }
                }

                Ok(Event::Eof) | Err(_) => {
                    break;
                }

                _ => {}
            }
        }

        Err("Bad XML")
    }
}

pub struct RuntimeCharacteristics<'a> {
    root: std::slice::Iter<'a, Characteristic>,
}

impl<'a> Iterator for RuntimeCharacteristics<'a> {
    type Item = &'a Characteristic;
    fn next(&mut self) -> Option<&'a Characteristic> {
        while let Some(child) = self.root.next() {
            if child.characteristic_type == "NODE" {
                return Some(&child);
            }
        }

        None
    }
}

pub fn read_characteristic<R>(
    xml_reader: &mut Reader<R>,
    buf: &mut Vec<u8>,
    e: &BytesStart,
    level: i32,
) -> Option<Characteristic>
where
    R: std::io::BufRead,
{
    let mut characteristic_type: Option<String> = None;

    for attribute in e.attributes() {
        if let Ok(attribute) = attribute {
            if attribute.key.as_ref() == b"type" {
                if let Ok(value) = attribute.unescape_value() {
                    characteristic_type.replace(value.into_owned());
                }
            }
        }
    }

    let mut level = level;

    let mut characteristic_parameters = Vec::new();
    let mut child_characteristics = Vec::new();

    let mut handle_element = |xml_reader: &mut Reader<R>, e: &BytesStart, level: i32| -> bool {
        if e.name().as_ref().eq_ignore_ascii_case(b"characteristic") {
            let mut buf = Vec::new();
            if let Some(characteristic) = read_characteristic(xml_reader, &mut buf, e, level) {
                child_characteristics.push(characteristic);
            }
            return true;
        } else if e.name().as_ref().eq_ignore_ascii_case(b"parm") {
            let mut buf = Vec::new();
            if let Some(parameter) = read_parm(xml_reader, &mut buf, e, level) {
                characteristic_parameters.push(parameter);
            }
            return true;
        }
        false
    };

    if level > 0 {
        loop {
            match xml_reader.read_event_into(buf) {
                Ok(Event::Start(ref e)) => {
                    if !handle_element(xml_reader, e, 1) {
                        level += 1;
                    }
                }

                Ok(Event::Empty(ref e)) => {
                    handle_element(xml_reader, e, 0);
                }

                Ok(Event::End(_)) => {
                    level -= 1;
                    if level == 0 {
                        break;
                    }
                }

                Ok(Event::Eof) | Err(_) => {
                    break;
                }

                _ => {}
            }
        }
    }

    if let Some(characteristic_type) = characteristic_type {
        return Some(Characteristic {
            characteristic_type,
            characteristic_parameters,
            child_characteristics,
        });
    }

    buf.clear();

    None
}

pub fn write_characteristic<W>(
    xml_writer: &mut Writer<W>,
    characteristic: &Characteristic,
) -> quick_xml::Result<()>
where
    W: Write,
{
    let mut elem = BytesStart::new("characteristic");

    elem.push_attribute(("type", characteristic.characteristic_type.as_str()));

    xml_writer.write_event(Event::Start(elem))?;

    for parameter in &characteristic.characteristic_parameters {
        write_parm(xml_writer, &parameter)?;
    }

    for characteristic in &characteristic.child_characteristics {
        write_characteristic(xml_writer, &characteristic)?;
    }

    xml_writer.write_event(Event::End(BytesEnd::new("characteristic")))?;

    Ok(())
}
