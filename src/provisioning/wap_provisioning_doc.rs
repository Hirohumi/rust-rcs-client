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

use std::{io::Write, str::FromStr};

use quick_xml::{
    events::{BytesEnd, BytesStart, Event},
    Reader, Writer,
};

use rust_rcs_core::{ffi::log::platform_log, util::raw_string::StrEq};

use super::characteristic::{read_characteristic, write_characteristic, Characteristic, Parameter};

const LOG_TAG: &str = "wap_doc";

pub const DEFAULT_APPLICATION_PORT: u16 = 37273;

pub struct WapProvisioningDoc {
    root: Vec<Characteristic>,
}

impl WapProvisioningDoc {
    pub fn new(root: Vec<Characteristic>) -> WapProvisioningDoc {
        WapProvisioningDoc { root }
    }

    fn get_child_characteristic(&self, name: &str) -> Option<&Characteristic> {
        for child in &self.root {
            if child.characteristic_type == name {
                return Some(child);
            }
        }

        None
    }

    pub fn get_vers_token(&self) -> Option<(i64, i64, Option<(&str, Option<i64>)>)> {
        if let Some(vers) = self.get_child_characteristic("VERS") {
            if let (Some(version), Some(version_validity)) = (
                vers.get_parameter("version"),
                vers.get_parameter("validity"),
            ) {
                if let (Ok(version), Ok(version_validity)) =
                    (version.parse::<i64>(), version_validity.parse::<i64>())
                {
                    if (version > 0 && version_validity > 0) || version <= 0 {
                        if let Some(token) = self.get_child_characteristic("TOKEN") {
                            if let Some(token_string) = token.get_parameter("token") {
                                if let Some(token_validity) = token.get_parameter("validity") {
                                    if let Ok(token_validity) = token_validity.parse::<i64>() {
                                        return Some((
                                            version,
                                            version_validity,
                                            Some((token_string, Some(token_validity))),
                                        ));
                                    }
                                }

                                return Some((
                                    version,
                                    version_validity,
                                    Some((token_string, None)),
                                ));
                            }
                        }
                        return Some((version, version_validity, None));
                    }
                }
            }
        }

        None
    }

    pub fn access_control(&self) -> Option<AccessControl> {
        if let Some(access_control) = self.get_child_characteristic("ACCESS-CONTROL") {
            return Some(AccessControl {
                root: access_control.child_characteristics.iter(),
            });
        }

        None
    }

    pub fn get_user_msisdn(&self) -> Option<&str> {
        if let Some(user) = self.get_child_characteristic("User") {
            return user.get_parameter("msisdn");
        }

        None
    }

    pub fn get_msg(&self) -> Option<&Characteristic> {
        self.get_child_characteristic("MSG")
    }

    pub fn get_sms_policy(&self) -> Option<u16> {
        if self.root.len() == 1 {
            if let Some(policy) = self.get_child_characteristic("POLICY") {
                if let Some(sms_port) = policy.get_parameter("SMS_port") {
                    if let Ok(port) = sms_port.parse::<u16>() {
                        return Some(port);
                    }
                }
                return Some(DEFAULT_APPLICATION_PORT);
            }
        }

        None
    }

    pub fn children(&self) -> std::slice::Iter<Characteristic> {
        self.root.iter()
    }

    pub fn applications(&self) -> Applications {
        Applications {
            root: self.root.iter(),
        }
    }

    pub fn get_application_characteristic(&self, app_id: &str) -> Option<&Characteristic> {
        for child in &self.root {
            if child.characteristic_type == "APPLICATION" {
                if let Some(app_id_parm) = child.get_parameter("AppID") {
                    if app_id_parm == app_id {
                        return Some(child);
                    }
                }
            }
        }

        None
    }

    pub fn update_application(&mut self, app_id: &str, application: &Characteristic) {
        platform_log(LOG_TAG, format!("update application {}", app_id));

        for child in self.root.iter_mut() {
            if child.characteristic_type == "APPLICATION" {
                if let Some(app_id_parm) = child.get_parameter("AppID") {
                    if app_id_parm == app_id {
                        child.characteristic_parameters =
                            application.characteristic_parameters.clone();
                        child.child_characteristics = application.child_characteristics.clone();
                        return;
                    }
                }
            }
        }

        platform_log(LOG_TAG, format!("inserting application {}", app_id));

        self.root.push(application.clone())
    }

    pub fn remove_non_application_characteristics(&mut self) {
        self.root
            .retain(|child| child.characteristic_type == "APPLICATION");
    }

    pub fn add_non_application_characteristic(&mut self, characteristic: &Characteristic) {
        self.root.push(characteristic.clone())
    }
}

pub fn read_wap_provisioning_doc<R>(
    xml_reader: &mut Reader<R>,
    buf: &mut Vec<u8>,
    _e: &BytesStart,
) -> WapProvisioningDoc
where
    R: std::io::BufRead,
{
    let mut level = 1;

    let mut root = Vec::new();

    let mut handle_element = |xml_reader: &mut Reader<R>, e: &BytesStart, level: i32| -> bool {
        if e.name().as_ref().equals_bytes(b"characteristic", true) {
            let mut buf = Vec::new();
            if let Some(characteristic) = read_characteristic(xml_reader, &mut buf, e, level) {
                root.push(characteristic);
            }
            return true;
        }
        false
    };

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

    buf.clear();

    WapProvisioningDoc { root }
}

pub fn write_wap_provisioning_doc<W>(
    xml_writer: &mut Writer<W>,
    wap_provisioning_doc: &WapProvisioningDoc,
) -> quick_xml::Result<()>
where
    W: Write,
{
    let elem = BytesStart::new("wap-provisioningdoc");

    xml_writer.write_event(Event::Start(elem))?;

    for characteristic in &wap_provisioning_doc.root {
        write_characteristic(xml_writer, &characteristic)?;
    }

    xml_writer.write_event(Event::End(BytesEnd::new("wap-provisioningdoc")))?;

    Ok(())
}

impl FromStr for WapProvisioningDoc {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut xml_reader = Reader::from_str(s);
        let mut buf = Vec::new();
        loop {
            match xml_reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    if e.name().as_ref().equals_bytes(b"wap-provisioningdoc", true) {
                        let mut buf = Vec::new();
                        return Ok(read_wap_provisioning_doc(&mut xml_reader, &mut buf, e));
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

pub struct Applications<'a> {
    root: std::slice::Iter<'a, Characteristic>,
}

impl<'a> Iterator for Applications<'a> {
    type Item = &'a Characteristic;
    fn next(&mut self) -> Option<&'a Characteristic> {
        while let Some(characteristic) = self.root.next() {
            platform_log(
                LOG_TAG,
                format!("iterating ROOT, {}", characteristic.characteristic_type),
            );
            if characteristic.characteristic_type == "APPLICATION" {
                return Some(characteristic);
            }
        }

        None
    }
}

pub struct AccessControl<'a> {
    root: std::slice::Iter<'a, Characteristic>,
}

impl<'a> Iterator for AccessControl<'a> {
    type Item = AccessControlInfo<'a>;
    fn next(&mut self) -> Option<AccessControlInfo<'a>> {
        while let Some(characteristic) = self.root.next() {
            platform_log(
                LOG_TAG,
                format!(
                    "iterating ACCESS-CONTROL, {}",
                    characteristic.characteristic_type
                ),
            );
            if characteristic.characteristic_type == "DEFAULT"
                || characteristic.characteristic_type == "SERVER"
            {
                if let Some(fqdn) = characteristic.get_parameter("fqdn") {
                    return Some(AccessControlInfo {
                        is_default_server: characteristic.characteristic_type == "DEFAULT",
                        fqdn,
                        is_id_provider: if let Some(id_provider) =
                            characteristic.get_parameter("id-provider")
                        {
                            if id_provider == "1" {
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        },
                        root: characteristic,
                    });
                } else {
                    platform_log(LOG_TAG, "cannot find parm fqdn");
                }
            }
        }

        None
    }
}

pub struct AccessControlInfo<'a> {
    pub is_default_server: bool,
    pub fqdn: &'a str,
    pub is_id_provider: bool,
    root: &'a Characteristic,
}

impl<'a> AccessControlInfo<'a> {
    pub fn app_ids(&self) -> AppIDs {
        AppIDs {
            root: self.root.characteristic_parameters.iter(),
        }
    }
}

pub struct AppIDs<'a> {
    root: std::slice::Iter<'a, Parameter>,
}

impl<'a> Iterator for AppIDs<'a> {
    type Item = &'a str;
    fn next(&mut self) -> Option<&'a str> {
        while let Some(parameter) = self.root.next() {
            if parameter.name == "app-id" {
                return Some(&parameter.value);
            }
        }

        None
    }
}
