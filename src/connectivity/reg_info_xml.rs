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

use quick_xml::{
    events::{BytesStart, Event},
    Reader,
};

pub struct RegInfoXml {
    pub version: String,
    pub state: String,
    pub registration_nodes: Vec<RegistrationNode>,
}

pub struct RegistrationNode {
    pub aor: String,
    pub id: String,
    pub state: String,
    pub contact_nodes: Vec<ContactNode>,
}

pub struct ContactNode {
    pub state: String,
    pub event: String,
    pub duration_registered: Option<String>,
    pub expires: Option<String>,
    pub id: String,
    pub uri: String,
}

pub fn parse_xml(data: &[u8]) -> Option<RegInfoXml> {
    let mut xml_reader = Reader::from_reader(data);

    let mut buf = Vec::new();
    loop {
        match xml_reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                if e.name().as_ref().eq_ignore_ascii_case(b"reginfo") {
                    let mut buf = Vec::new();
                    return parse_reg_info(&mut xml_reader, &mut buf, e, 1);
                }
            }

            Ok(Event::Empty(ref e)) => {
                if e.name().as_ref().eq_ignore_ascii_case(b"reginfo") {
                    let mut buf = Vec::new();
                    return parse_reg_info(&mut xml_reader, &mut buf, e, 0);
                }
            }

            Ok(Event::Eof) | Err(_) => {
                break;
            }

            _ => {}
        }
    }

    None
}

pub fn parse_reg_info<R>(
    xml_reader: &mut Reader<R>,
    buf: &mut Vec<u8>,
    e: &BytesStart,
    level: i32,
) -> Option<RegInfoXml>
where
    R: std::io::BufRead,
{
    let mut version: Option<String> = None;
    let mut state: Option<String> = None;

    for attribute in e.attributes() {
        if let Ok(attribute) = attribute {
            if attribute.key.as_ref() == b"version" {
                if let Ok(attribute_value) = attribute.unescape_value() {
                    version.replace(attribute_value.into_owned());
                }
            } else if attribute.key.as_ref() == b"state" {
                if let Ok(attribute_value) = attribute.unescape_value() {
                    state.replace(attribute_value.into_owned());
                }
            }
        }
    }

    let mut registration_nodes = Vec::new();

    let mut handle_element = |xml_reader: &mut Reader<R>, e: &BytesStart, level: i32| -> bool {
        if e.name().as_ref().eq_ignore_ascii_case(b"registration") {
            let mut buf = Vec::new();
            if let Some(registration_node) = parse_registration_node(xml_reader, &mut buf, e, level)
            {
                registration_nodes.push(registration_node);
            }
            return true;
        }
        false
    };

    let mut level = level;

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

    if let (Some(version), Some(state)) = (version, state) {
        return Some(RegInfoXml {
            version,
            state,
            registration_nodes,
        });
    }

    None
}

pub fn parse_registration_node<R>(
    xml_reader: &mut Reader<R>,
    buf: &mut Vec<u8>,
    e: &BytesStart,
    level: i32,
) -> Option<RegistrationNode>
where
    R: std::io::BufRead,
{
    let mut aor: Option<String> = None;
    let mut id: Option<String> = None;
    let mut state: Option<String> = None;

    for attribute in e.attributes() {
        if let Ok(attribute) = attribute {
            if attribute.key.as_ref() == b"aor" {
                if let Ok(attribute_value) = attribute.unescape_value() {
                    aor.replace(attribute_value.into_owned());
                }
            } else if attribute.key.as_ref() == b"id" {
                if let Ok(attribute_value) = attribute.unescape_value() {
                    id.replace(attribute_value.into_owned());
                }
            } else if attribute.key.as_ref() == b"state" {
                if let Ok(attribute_value) = attribute.unescape_value() {
                    state.replace(attribute_value.into_owned());
                }
            }
        }
    }

    let mut contact_nodes = Vec::new();

    let mut level = level;

    let mut handle_element = |xml_reader: &mut Reader<R>, e: &BytesStart, level: i32| -> bool {
        if e.name().as_ref().eq_ignore_ascii_case(b"contact") {
            let mut buf = Vec::new();
            if let Some(parameter) = parse_contact_node(xml_reader, &mut buf, e, level) {
                contact_nodes.push(parameter);
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

    if let (Some(aor), Some(id), Some(state)) = (aor, id, state) {
        return Some(RegistrationNode {
            aor,
            id,
            state,
            contact_nodes,
        });
    }

    None
}

pub fn parse_contact_node<R>(
    xml_reader: &mut Reader<R>,
    buf: &mut Vec<u8>,
    e: &BytesStart,
    level: i32,
) -> Option<ContactNode>
where
    R: std::io::BufRead,
{
    let mut state: Option<String> = None;
    let mut event: Option<String> = None;
    let mut duration_registered: Option<String> = None;
    let mut expires: Option<String> = None;
    let mut id: Option<String> = None;

    for attribute in e.attributes() {
        if let Ok(attribute) = attribute {
            if attribute.key.as_ref() == b"state" {
                if let Ok(attribute_value) = attribute.unescape_value() {
                    state.replace(attribute_value.into_owned());
                }
            } else if attribute.key.as_ref() == b"event" {
                if let Ok(attribute_value) = attribute.unescape_value() {
                    event.replace(attribute_value.into_owned());
                }
            } else if attribute.key.as_ref() == b"duration-registered" {
                if let Ok(attribute_value) = attribute.unescape_value() {
                    duration_registered.replace(attribute_value.into_owned());
                }
            } else if attribute.key.as_ref() == b"expires" {
                if let Ok(attribute_value) = attribute.unescape_value() {
                    expires.replace(attribute_value.into_owned());
                }
            } else if attribute.key.as_ref() == b"id" {
                if let Ok(attribute_value) = attribute.unescape_value() {
                    id.replace(attribute_value.into_owned());
                }
            }
        }
    }

    let mut uri: Option<String> = None;

    let mut level = level;

    if level > 0 {
        loop {
            match xml_reader.read_event_into(buf) {
                Ok(Event::Start(ref e)) => {
                    if e.name().as_ref().eq_ignore_ascii_case(b"uri") {
                        let mut buf = Vec::new();
                        if let Some(contact_node_uri) = parse_contact_node_uri(xml_reader, &mut buf)
                        {
                            uri.replace(contact_node_uri);
                        }
                    } else {
                        level += 1;
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

    if let (Some(state), Some(event), Some(id), Some(uri)) = (state, event, id, uri) {
        return Some(ContactNode {
            state,
            event,
            duration_registered,
            expires,
            id,
            uri,
        });
    }

    None
}

pub fn parse_contact_node_uri<R>(xml_reader: &mut Reader<R>, buf: &mut Vec<u8>) -> Option<String>
where
    R: std::io::BufRead,
{
    let mut uri: Option<String> = None;

    let mut level = 1;

    loop {
        match xml_reader.read_event_into(buf) {
            Ok(Event::Start(ref _e)) => {
                level += 1;
            }

            Ok(Event::Text(ref e)) => {
                if level == 1 {
                    if let Ok(t) = e.unescape() {
                        uri.replace(t.into_owned());
                    }
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

    uri
}
