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

extern crate chrono;
extern crate quick_xml;

use std::{fs::File, io::Write};

use chrono::{DateTime, Duration, FixedOffset, Utc};

use quick_xml::{
    events::{BytesEnd, BytesStart, Event},
    Reader, Writer,
};

use rust_rcs_core::{ffi::log::platform_log, util::raw_string::StrEq};

use super::{
    characteristic::Characteristic,
    wap_provisioning_doc::{
        read_wap_provisioning_doc, write_wap_provisioning_doc, WapProvisioningDoc,
    },
};

const LOG_TAG: &str = "provisioning-doc";

pub struct LocalProvisioningDoc {
    root: Vec<ProvisiongingInfo>,
}

impl LocalProvisioningDoc {
    pub fn new(root: Vec<ProvisiongingInfo>) -> LocalProvisioningDoc {
        LocalProvisioningDoc { root }
    }

    pub fn get_provisioning_info(
        &self,
        get_default: bool,
        id: &str,
    ) -> Option<(
        i64,
        DateTime<FixedOffset>,
        Option<(&str, DateTime<FixedOffset>)>,
        Option<&WapProvisioningDoc>,
    )> {
        for provisioning_info in &self.root {
            if (get_default && provisioning_info.default == "yes") || provisioning_info.id == id {
                return destructure_provisioning_info(&provisioning_info);
            }
        }

        None
    }

    pub fn unlink(mut self, is_default: bool, id: &str) -> Self {
        self.root.retain(|provisioning_info| -> bool {
            if (is_default && provisioning_info.default == "yes") || provisioning_info.id == id {
                return false;
            }

            true
        });

        self
    }

    pub fn unlink_token(mut self, is_default: bool, id: &str) -> Self {
        for provisioning_info in self.root.iter_mut() {
            if (is_default && provisioning_info.default == "yes") || provisioning_info.id == id {
                if provisioning_info.default == "yes" {
                    provisioning_info.TOKEN_token.take();
                    provisioning_info.TOKEN_validity_through.take();
                    continue;
                }
            }
        }

        self
    }

    pub fn update_application(
        mut self,
        is_default: bool,
        id: &str,
        app_id: &str,
        application: &Characteristic,
    ) -> Self {
        platform_log(
            LOG_TAG,
            format!("update application {} for server {}", app_id, id),
        );

        for provisioning_info in &mut self.root {
            if (is_default && provisioning_info.default == "yes") || provisioning_info.id == id {
                if let Some(ref mut wap_provisioning_doc) = &mut provisioning_info.wap_doc {
                    wap_provisioning_doc.update_application(app_id, application);
                } else {
                    platform_log(
                        LOG_TAG,
                        format!("inserting application {} for server {}", app_id, id),
                    );

                    let mut v = Vec::new();

                    v.push(application.clone());

                    let wap_provisioning_doc = WapProvisioningDoc::new(v);

                    provisioning_info.wap_doc.replace(wap_provisioning_doc);
                }

                continue;
            }
        }

        self
    }

    pub fn update_vers_token(
        &mut self,
        is_default: bool,
        id: &str,
        vers: i64,
        vers_validity_through: String,
        token: Option<(&str, Option<i64>)>,
    ) {
        let update_token = |provisioning_info: &mut ProvisiongingInfo,
                            token: Option<(&str, Option<i64>)>| {
            if let Some((token_string, token_validity)) = token {
                provisioning_info
                    .TOKEN_token
                    .replace(String::from(token_string));

                if let Some(token_validity) = token_validity {
                    if token_validity > 0 {
                        let token_validity_through =
                            (Utc::now() + Duration::seconds(token_validity)).to_rfc3339();
                        provisioning_info
                            .TOKEN_validity_through
                            .replace(token_validity_through);
                    } else {
                        let token_validity_through =
                            (Utc::now() + Duration::days(365)).to_rfc3339();
                        provisioning_info
                            .TOKEN_validity_through
                            .replace(token_validity_through);
                    }
                } else {
                    let token_validity_through = (Utc::now() + Duration::days(365)).to_rfc3339();
                    provisioning_info
                        .TOKEN_validity_through
                        .replace(token_validity_through);
                }
            } else {
                provisioning_info.TOKEN_token.take();
                provisioning_info.TOKEN_validity_through.take();
            }
        };

        for provisioning_info in &mut self.root {
            if (is_default && provisioning_info.default == "yes") || provisioning_info.id == id {
                provisioning_info.VERS_version = format!("{}", vers);
                provisioning_info.VERS_validity_through = format!("{}", vers_validity_through);
                update_token(provisioning_info, token);
                return;
            }
        }

        let mut provisioning_info = ProvisiongingInfo {
            default: if is_default {
                String::from("yes")
            } else {
                String::from("no")
            },
            id: String::from(id),
            VERS_version: format!("{}", vers),
            VERS_validity_through: vers_validity_through,
            TOKEN_token: None,
            TOKEN_validity_through: None,
            wap_doc: None,
        };

        update_token(&mut provisioning_info, token);

        self.root.push(provisioning_info);
    }

    pub fn remove_non_application_characteristics_for_default_server(&mut self) {
        for provisioning_info in &mut self.root {
            if provisioning_info.default == "yes" {
                if let Some(wap_provisioning_doc) = &mut provisioning_info.wap_doc {
                    wap_provisioning_doc.remove_non_application_characteristics();
                }
                break;
            }
        }
    }

    pub fn update_characteristics_for_default_server(&mut self, characteristic: &Characteristic) {
        for provisioning_info in &mut self.root {
            if provisioning_info.default == "yes" {
                if let Some(wap_provisioning_doc) = &mut provisioning_info.wap_doc {
                    wap_provisioning_doc.add_non_application_characteristic(characteristic);
                } else {
                    let v = Vec::new();
                    let mut wap_provisioning_doc = WapProvisioningDoc::new(v);
                    wap_provisioning_doc.add_non_application_characteristic(characteristic);
                    provisioning_info.wap_doc.replace(wap_provisioning_doc);
                }
                break;
            }
        }
    }

    pub fn provisioning_files(&self) -> ProvisiongingFiles {
        ProvisiongingFiles {
            root: self.root.iter(),
        }
    }
}

pub fn read_provisionging_info<R>(
    xml_reader: &mut Reader<R>,
    buf: &mut Vec<u8>,
    e: &BytesStart,
) -> Option<ProvisiongingInfo>
where
    R: std::io::BufRead,
{
    let mut level = 1;

    let mut default: Option<String> = None;
    let mut id: Option<String> = None;
    let mut VERS_version: Option<String> = None;
    let mut VERS_validity_through: Option<String> = None;
    let mut TOKEN_token: Option<String> = None;
    let mut TOKEN_validity_through: Option<String> = None;

    let mut wap_provisioning_doc: Option<WapProvisioningDoc> = None;

    for attribute in e.attributes() {
        if let Ok(attribute) = attribute {
            if attribute.key.as_ref() == b"default" {
                if let Ok(attribute_value) = attribute.unescape_value() {
                    default.replace(attribute_value.into_owned());
                }
            } else if attribute.key.as_ref() == b"id" {
                if let Ok(attribute_value) = attribute.unescape_value() {
                    id.replace(attribute_value.into_owned());
                }
            } else if attribute.key.as_ref() == b"VERS_version" {
                if let Ok(attribute_value) = attribute.unescape_value() {
                    VERS_version.replace(attribute_value.into_owned());
                }
            } else if attribute.key.as_ref() == b"VERS_validity_through" {
                if let Ok(attribute_value) = attribute.unescape_value() {
                    VERS_validity_through.replace(attribute_value.into_owned());
                }
            } else if attribute.key.as_ref() == b"TOKEN_token" {
                if let Ok(attribute_value) = attribute.unescape_value() {
                    TOKEN_token.replace(attribute_value.into_owned());
                }
            } else if attribute.key.as_ref() == b"TOKEN_validity_through" {
                if let Ok(attribute_value) = attribute.unescape_value() {
                    TOKEN_validity_through.replace(attribute_value.into_owned());
                }
            }
        }
    }

    loop {
        match xml_reader.read_event_into(buf) {
            Ok(Event::Start(ref e)) => {
                level += 1;
                if e.name().as_ref().equals_bytes(b"wap-provisioningdoc", true) {
                    let mut buf = Vec::new();
                    wap_provisioning_doc
                        .replace(read_wap_provisioning_doc(xml_reader, &mut buf, e));
                    level -= 1;
                }
            }

            Ok(Event::End(_)) => {
                level -= 1;
                if level == 0 {
                    break;
                }
            }

            Ok(Event::Eof) | Err(_) => {
                return None;
            }

            _ => {}
        }
    }

    buf.clear();

    if let (Some(default), Some(id), Some(VERS_version), Some(VERS_validity_through)) =
        (default, id, VERS_version, VERS_validity_through)
    {
        return Some(ProvisiongingInfo {
            default,
            id,
            VERS_version,
            VERS_validity_through,
            TOKEN_token,
            TOKEN_validity_through,
            wap_doc: wap_provisioning_doc,
        });
    }

    None
}

pub fn read_provisionging_doc<R>(
    xml_reader: &mut Reader<R>,
    buf: &mut Vec<u8>,
    e: &BytesStart,
) -> LocalProvisioningDoc
where
    R: std::io::BufRead,
{
    let mut level = 1;

    let mut root = Vec::new();

    loop {
        match xml_reader.read_event_into(buf) {
            Ok(Event::Start(ref e)) => {
                level += 1;
                if e.name().as_ref().equals_bytes(b"provisioning-info", true) {
                    let mut buf = Vec::new();
                    if let Some(provisioning_info) =
                        read_provisionging_info(xml_reader, &mut buf, e)
                    {
                        root.push(provisioning_info);
                    }
                    level -= 1;
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

    buf.clear();

    LocalProvisioningDoc { root }
}

pub fn load_provisionging_doc(path: &str) -> Option<LocalProvisioningDoc> {
    if let Ok(mut xml_reader) = Reader::from_file(path) {
        let mut buf = Vec::new();
        loop {
            match xml_reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    if e.name().as_ref().equals_bytes(b"provisioning-doc", true) {
                        let mut buf = Vec::new();
                        return Some(read_provisionging_doc(&mut xml_reader, &mut buf, e));
                    }
                }

                Ok(Event::Eof) | Err(_) => {
                    break;
                }

                _ => {}
            }
        }
    }

    None
}

pub fn write_provisioning_info<W>(
    xml_writer: &mut Writer<W>,
    provisioning_info: &ProvisiongingInfo,
) -> quick_xml::Result<()>
where
    W: Write,
{
    let mut elem = BytesStart::new("provisioning-info");

    elem.push_attribute(("default", provisioning_info.default.as_str()));
    elem.push_attribute(("id", provisioning_info.id.as_str()));
    elem.push_attribute(("VERS_version", provisioning_info.VERS_version.as_str()));
    elem.push_attribute((
        "VERS_validity_through",
        provisioning_info.VERS_validity_through.as_str(),
    ));

    if let (Some(TOKEN_token), Some(TOKEN_validity_through)) = (
        &provisioning_info.TOKEN_token,
        &provisioning_info.TOKEN_validity_through,
    ) {
        elem.push_attribute(("TOKEN_token", TOKEN_token.as_str()));
        elem.push_attribute(("TOKEN_validity_through", TOKEN_validity_through.as_str()));
    }

    xml_writer.write_event(Event::Start(elem))?;

    if let Some(wap_provisioning_doc) = &provisioning_info.wap_doc {
        write_wap_provisioning_doc(xml_writer, wap_provisioning_doc)?;
    }

    xml_writer.write_event(Event::End(BytesEnd::new("provisioning-info")))?;

    Ok(())
}

pub fn write_provisionging_doc<W>(
    xml_writer: &mut Writer<W>,
    provisioning_doc: &LocalProvisioningDoc,
) -> quick_xml::Result<()>
where
    W: Write,
{
    let elem = BytesStart::new("provisioning-doc");

    xml_writer.write_event(Event::Start(elem))?;

    for provisioning_info in &provisioning_doc.root {
        write_provisioning_info(xml_writer, provisioning_info)?;
    }

    xml_writer.write_event(Event::End(BytesEnd::new("provisioning-doc")))?;

    Ok(())
}

pub fn store_provisionging_doc(
    provisioning_doc: &LocalProvisioningDoc,
    path: &str,
) -> Result<(), ()> {
    if let Ok(f) = File::create(path) {
        let mut xml_writer = Writer::new(f);
        if let Ok(_) = write_provisionging_doc(&mut xml_writer, provisioning_doc) {
            return Ok(());
        }
    }

    Err(())
}

pub struct ProvisiongingInfo {
    pub default: String,
    pub id: String,
    pub VERS_version: String,
    pub VERS_validity_through: String,
    pub TOKEN_token: Option<String>,
    pub TOKEN_validity_through: Option<String>,
    pub wap_doc: Option<WapProvisioningDoc>,
}

fn destructure_provisioning_info(
    provisioning_info: &ProvisiongingInfo,
) -> Option<(
    i64,
    DateTime<FixedOffset>,
    Option<(&str, DateTime<FixedOffset>)>,
    Option<&WapProvisioningDoc>,
)> {
    platform_log(LOG_TAG, "de-structuring provisioning-info");

    if let (Ok(vers_version), Ok(vers_validity_through)) = (
        provisioning_info.VERS_version.parse::<i64>(),
        DateTime::parse_from_rfc3339(&provisioning_info.VERS_validity_through),
    ) {
        if let (Some(token_string), Some(token_validity_through)) = (
            &provisioning_info.TOKEN_token,
            &provisioning_info.TOKEN_validity_through,
        ) {
            if let (Ok(token_validity_through),) =
                (DateTime::parse_from_rfc3339(&token_validity_through),)
            {
                if let Some(wap_doc) = &provisioning_info.wap_doc {
                    return Some((
                        vers_version,
                        vers_validity_through,
                        Some((&token_string, token_validity_through)),
                        Some(wap_doc),
                    ));
                } else {
                    return Some((
                        vers_version,
                        vers_validity_through,
                        Some((&token_string, token_validity_through)),
                        None,
                    ));
                }
            }
        } else {
            if let Some(wap_doc) = &provisioning_info.wap_doc {
                return Some((vers_version, vers_validity_through, None, Some(wap_doc)));
            } else {
                return Some((vers_version, vers_validity_through, None, None));
            }
        }
    }

    None
}

pub struct ProvisiongingFiles<'a> {
    root: std::slice::Iter<'a, ProvisiongingInfo>,
}

impl<'a> Iterator for ProvisiongingFiles<'a> {
    type Item = &'a WapProvisioningDoc;
    fn next(&mut self) -> Option<&'a WapProvisioningDoc> {
        while let Some(provisioning_info) = self.root.next() {
            if let Some(wap_doc) = &provisioning_info.wap_doc {
                return Some(wap_doc);
            }
        }

        None
    }
}
