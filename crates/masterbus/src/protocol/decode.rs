//! Decode raw frames into messages and field values.

use super::{BTM1_META_ADDR_FLAG, MbFrame, MbMessage, VisualizationType, can_class, meta_op};
use crate::value::{Date, Time, Value};

/// Decode a raw `(raw_can_id, data)` pair into an [`MbFrame`].
///
/// The SocketCAN `raw_id()` carries flag bits in the top 3 bits; we mask to the
/// 29-bit id, then split class (bits 28:24) and device address (bits 23:0).
pub fn frame_from_raw(raw_can_id: u32, data: &[u8]) -> MbFrame {
    let can_id = raw_can_id & 0x1FFF_FFFF;
    MbFrame {
        device_addr: can_id & 0x00_FF_FF_FF,
        can_class: ((can_id >> 24) & 0x1F) as u8,
        data: data.to_vec(),
    }
}

/// Parse an [`MbFrame`] into a structured [`MbMessage`].
pub fn parse_frame(frame: &MbFrame) -> MbMessage {
    let addr = frame.device_addr;
    let data = &frame.data;
    match frame.can_class {
        can_class::DEVICE_BROADCAST => parse_broadcast(addr, data),
        can_class::MONITORING_DATA | can_class::MONITORING_REQ => {
            parse_monitoring(addr, frame.can_class, data)
        }
        other => MbMessage::Unknown {
            device_addr: addr,
            can_class: other,
            data: data.to_vec(),
        },
    }
}

fn parse_broadcast(device_addr: u32, data: &[u8]) -> MbMessage {
    if data.len() < 6 {
        return MbMessage::Unknown {
            device_addr,
            can_class: can_class::DEVICE_BROADCAST,
            data: data.to_vec(),
        };
    }
    MbMessage::DeviceBroadcast {
        device_addr,
        type_code: data[0],
        instance: data[3],
        firmware_version: u16::from_le_bytes([data[4], data[5]]),
    }
}

fn parse_monitoring(device_addr: u32, class: u8, data: &[u8]) -> MbMessage {
    if data.len() < 2 {
        return MbMessage::Unknown {
            device_addr,
            can_class: class,
            data: data.to_vec(),
        };
    }
    let field_index = data[0];
    let tab_index = data[1];
    // 2 bytes = request; 6+ bytes = value. A 3-byte frame is a *write* command
    // (ours or another master's) and must not be mistaken for a value response
    // (our own writes loop back on the local socket).
    if data.len() == 2 {
        MbMessage::MonitoringReq {
            device_addr,
            field_index,
            tab_index,
        }
    } else if data.len() >= 6 {
        MbMessage::MonitoringData {
            device_addr,
            field_index,
            tab_index,
            raw: data[2..].to_vec(),
        }
    } else {
        MbMessage::Unknown {
            device_addr,
            can_class: class,
            data: data.to_vec(),
        }
    }
}

/// Decode a 4-byte value payload into a [`Value`] per the field's visualization.
pub fn decode_value(raw: &[u8], viz: VisualizationType) -> Value {
    use VisualizationType as V;
    match viz {
        V::Float | V::GrayVisualization => {
            if raw.len() < 4 {
                return Value::Float(f32::NAN);
            }
            Value::Float(f32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
        }
        V::CheckBox | V::ToggleButton | V::PushButton => {
            // "On" may be a 1-byte 0x01 or a 4-byte float 1.0 (byte0=0); treat any
            // non-zero in the value as true.
            Value::Boolean(raw.iter().take(4).any(|&b| b != 0))
        }
        V::Time => {
            if raw.len() < 4 {
                return Value::Time(Time {
                    sec: -1,
                    min: 0,
                    hour: 0,
                    days: 0,
                });
            }
            let f = f32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
            if !f.is_finite() {
                return Value::Time(Time {
                    sec: -1,
                    min: 0,
                    hour: 0,
                    days: 0,
                });
            }
            let t = f as i64;
            Value::Time(Time {
                days: (t / 86_400) as u32,
                hour: ((t % 86_400) / 3_600) as i32,
                min: ((t % 3_600) / 60) as i32,
                sec: (t % 60) as i32,
            })
        }
        V::Date => {
            // Integer value packs the date as `year*416 + month*32 + day`.
            if raw.len() < 4 {
                return Value::Date(Date {
                    day: -1,
                    mon: -1,
                    year: -1,
                });
            }
            let f = f32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
            if !f.is_finite() {
                return Value::Date(Date {
                    day: -1,
                    mon: -1,
                    year: -1,
                });
            }
            let t = f as i64;
            Value::Date(Date {
                day: (t % 32) as i32,
                mon: ((t / 32) % 13) as i32,
                year: (t / 416) as i32,
            })
        }
        V::Radio | V::DropDown => {
            // The selection is the index as a 4-byte float (e.g. 1.0 = option 1).
            let index = if raw.len() >= 4 {
                f32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]).round() as i32
            } else {
                raw.first().map(|&b| b as i32).unwrap_or(0)
            };
            Value::List {
                index,
                options: Vec::new(),
            }
        }
        V::Eventable => Value::Eventable {
            index: raw.first().map(|&b| b as i32).unwrap_or(0),
            labels: Vec::new(),
        },
        V::DeviceList => Value::DeviceRef {
            // The selection is the index as a 4-byte float, same encoding as
            // Radio/DropDown (a small integer like 2.0 has a zero low byte, so
            // reading byte[0] would wrongly yield 0). `device_ids` is left empty
            // here — decode has no bus context; the caller resolves the index
            // against the address-sorted device list.
            index: if raw.len() >= 4 {
                f32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]).round() as i32
            } else {
                raw.first().map(|&b| b as i32).unwrap_or(0)
            },
            device_ids: Vec::new(),
        },
        V::Text => {
            // For Text-VIZ fields the field's "value" is the editable string
            // id (the Btm3 push carries it as f32; round to u16). The text
            // content lives at that sid in the device's string table — the
            // engine fills `text` via the chunk-read protocol before caching.
            if raw.len() < 4 {
                return Value::Text {
                    sid: 0,
                    text: String::new(),
                };
            }
            let f = f32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
            let sid = if f.is_finite() && f >= 0.0 && f <= u16::MAX as f32 {
                f.round() as u16
            } else {
                0
            };
            Value::Text {
                sid,
                text: String::new(),
            }
        }
    }
}

/// Compute the request/response matching key for an incoming frame that
/// belongs to the discovery protocol (property / schema / metadata).
/// Returns `None` for regular monitoring data (handled by the value layer).
pub fn waiter_key_for_frame(can_class_byte: u8, device_addr: u32, data: &[u8]) -> Option<String> {
    match can_class_byte {
        can_class::PROPERTY_INFO => {
            if data.len() < 2 {
                return None;
            }
            if data[0] == 0x30 && data.len() >= 4 {
                let str_id = u16::from_le_bytes([data[1], data[2]]);
                Some(format!(
                    "str:{:06X}:{:04X}:{}",
                    device_addr, str_id, data[3]
                ))
            } else {
                Some(format!(
                    "p:{:06X}:{:02X}:{:02X}",
                    device_addr, data[0], data[1]
                ))
            }
        }
        can_class::SCHEMA_DATA => schema_key("schema", device_addr, data),
        can_class::SCHEMA_DATA_ALARM => schema_key("schema_alarm", device_addr, data),
        // The history schema-response class is also used on the Btm1 metadata
        // address (`addr | 0x800000`) for headerless Btm3 monitoring pushes —
        // route only the real-addr form here; the value-push form is handled
        // by the reader.
        can_class::SCHEMA_DATA_HISTORY if device_addr & BTM1_META_ADDR_FLAG == 0 => {
            schema_key("schema_history", device_addr, data)
        }
        can_class::BTM3_META_DATA => {
            // Btm3 metadata response on the device's real address.
            if data.len() < 2 {
                return None;
            }
            if data[0] == meta_op::OPTION && data.len() >= 4 {
                Some(format!(
                    "btm3_meta:{:06X}:26:{}:{}",
                    device_addr, data[1], data[3]
                ))
            } else {
                Some(format!(
                    "btm3_meta:{:06X}:{:02X}:{}",
                    device_addr, data[0], data[1]
                ))
            }
        }
        can_class::MONITORING_DATA => {
            // Btm1 metadata response (Btm1-meta address bit set), or regular
            // monitoring data (handled by the value layer).
            if device_addr & BTM1_META_ADDR_FLAG != 0 {
                let real = device_addr & !BTM1_META_ADDR_FLAG;
                if data.len() < 2 {
                    return None;
                }
                if data[0] == meta_op::OPTION && data.len() >= 4 {
                    Some(format!("btm1_meta:{:06X}:26:{}:{}", real, data[1], data[3]))
                } else {
                    Some(format!(
                        "btm1_meta:{:06X}:{:02X}:{}",
                        real, data[0], data[1]
                    ))
                }
            } else {
                None
            }
        }
        // A Btm1 metadata "no value" reply (absent metadata / option). Routing
        // it to the same key as the request lets discovery resolve it instantly
        // instead of waiting out the timeout. Only the metadata-address form
        // is routed; a real-address `0x10` (e.g. a write ack) must NOT
        // masquerade as a value.
        can_class::WRITE_ACK if device_addr & BTM1_META_ADDR_FLAG != 0 => {
            let real = device_addr & !BTM1_META_ADDR_FLAG;
            if data.len() < 2 {
                return None;
            }
            if data[0] == meta_op::OPTION && data.len() >= 4 {
                Some(format!("btm1_meta:{:06X}:26:{}:{}", real, data[1], data[3]))
            } else {
                Some(format!(
                    "btm1_meta:{:06X}:{:02X}:{}",
                    real, data[0], data[1]
                ))
            }
        }
        _ => None,
    }
}

/// Build a schema-response waiter key with the channel-specific prefix.
fn schema_key(prefix: &str, device_addr: u32, data: &[u8]) -> Option<String> {
    if data.len() < 2 {
        return None;
    }
    match data[0] {
        0x03 if data.len() >= 4 => Some(format!(
            "{}:{:06X}:03:{}:{}",
            prefix, device_addr, data[1], data[3]
        )),
        0x03 => None,
        op => Some(format!(
            "{}:{:06X}:{:02X}:{}",
            prefix, device_addr, op, data[1]
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_decodes_packed_integer() {
        // 843000.0 = 2026*416 + 5*32 + 24  ->  24/05/2026
        let raw = 843000.0f32.to_le_bytes();
        assert_eq!(
            decode_value(&raw, VisualizationType::Date),
            Value::Date(Date {
                day: 24,
                mon: 5,
                year: 2026
            })
        );
    }

    #[test]
    fn device_ref_index_is_a_float_not_a_byte() {
        // An event-target value is the index as an f32 (2.0), whose low byte is
        // zero — reading byte[0] would wrongly give 0. Must decode to index 2.
        let raw = 2.0f32.to_le_bytes();
        assert_eq!(
            decode_value(&raw, VisualizationType::DeviceList),
            Value::DeviceRef {
                index: 2,
                device_ids: Vec::new()
            }
        );
    }

    #[test]
    fn wire_viz_0x09_is_device_list() {
        use crate::protocol::viz_from_wire;
        assert_eq!(viz_from_wire(0x09), VisualizationType::DeviceList);
    }

    #[test]
    fn time_decodes_total_seconds() {
        // 54496 s -> 0 days, 15:08:16
        let raw = 54496.0f32.to_le_bytes();
        assert_eq!(
            decode_value(&raw, VisualizationType::Time),
            Value::Time(Time {
                sec: 16,
                min: 8,
                hour: 15,
                days: 0
            })
        );
    }

    #[test]
    fn text_decodes_f32_as_sid() {
        // For Text-VIZ fields the field's value is f32(sid). From the wire on
        // the EasyView 5 (53A493) field 0x60 (Switch 1) the Btm3 push carried
        // bytes `00 00 80 40` = f32(4.0) and we verified the editable string
        // lives at sid 4. The decoder must round to u16 and leave text empty
        // for the engine to fill via the chunk-read protocol.
        let raw = 4.0f32.to_le_bytes();
        assert_eq!(
            decode_value(&raw, VisualizationType::Text),
            Value::Text {
                sid: 4,
                text: String::new()
            }
        );
        // Nav Chg Device-name push was f32(1.0) → sid 1.
        let raw = 1.0f32.to_le_bytes();
        assert_eq!(
            decode_value(&raw, VisualizationType::Text),
            Value::Text {
                sid: 1,
                text: String::new()
            }
        );
    }

    #[test]
    fn float_and_bool() {
        assert_eq!(
            decode_value(&9.0f32.to_le_bytes(), VisualizationType::Float),
            Value::Float(9.0)
        );
        assert_eq!(
            decode_value(&[1, 0, 0, 0], VisualizationType::CheckBox),
            Value::Boolean(true)
        );
        // "On" can also arrive as a 4-byte float 1.0 (byte0 = 0).
        assert_eq!(
            decode_value(&1.0f32.to_le_bytes(), VisualizationType::CheckBox),
            Value::Boolean(true)
        );
        assert_eq!(
            decode_value(&[0, 0, 0, 0], VisualizationType::CheckBox),
            Value::Boolean(false)
        );
    }
}
