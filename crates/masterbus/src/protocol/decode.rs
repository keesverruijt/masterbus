//! Decode raw frames into messages and field values.

use super::{can_class, shadow_op, MbFrame, MbMessage, VisualizationType, SHADOW_BIT};
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
        can_class::MONITORING_DATA | can_class::MONITORING_REQ => parse_monitoring(addr, frame.can_class, data),
        other => MbMessage::Unknown { device_addr: addr, can_class: other, data: data.to_vec() },
    }
}

fn parse_broadcast(device_addr: u32, data: &[u8]) -> MbMessage {
    if data.len() < 6 {
        return MbMessage::Unknown { device_addr, can_class: can_class::DEVICE_BROADCAST, data: data.to_vec() };
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
        return MbMessage::Unknown { device_addr, can_class: class, data: data.to_vec() };
    }
    let field_index = data[0];
    let tab_index = data[1];
    // 2 bytes = request; 6+ bytes = value. A 3-byte frame is a *write* command
    // (ours or another master's) and must not be mistaken for a value response
    // (our own writes loop back on the local socket).
    if data.len() == 2 {
        MbMessage::MonitoringReq { device_addr, field_index, tab_index }
    } else if data.len() >= 6 {
        MbMessage::MonitoringData { device_addr, field_index, tab_index, raw: data[2..].to_vec() }
    } else {
        MbMessage::Unknown { device_addr, can_class: class, data: data.to_vec() }
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
            Value::Boolean(raw.first().map(|&b| b != 0).unwrap_or(false))
        }
        V::Time => {
            if raw.len() < 4 {
                return Value::Time(Time { sec: -1, min: 0, hour: 0, days: 0 });
            }
            let f = f32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
            if !f.is_finite() {
                return Value::Time(Time { sec: -1, min: 0, hour: 0, days: 0 });
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
                return Value::Date(Date { day: -1, mon: -1, year: -1 });
            }
            let f = f32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
            if !f.is_finite() {
                return Value::Date(Date { day: -1, mon: -1, year: -1 });
            }
            let t = f as i64;
            Value::Date(Date {
                day: (t % 32) as i32,
                mon: ((t / 32) % 13) as i32,
                year: (t / 416) as i32,
            })
        }
        V::Radio | V::DropDown => {
            Value::List { index: raw.first().map(|&b| b as i32).unwrap_or(0), options: Vec::new() }
        }
        V::Eventable => {
            Value::Eventable { index: raw.first().map(|&b| b as i32).unwrap_or(0), labels: Vec::new() }
        }
        V::DeviceList => {
            Value::DeviceRef { index: raw.first().map(|&b| b as i32).unwrap_or(0), device_ids: Vec::new() }
        }
        V::Text => Value::Text(String::from_utf8_lossy(raw).into_owned()),
    }
}

/// Compute the request/response matching key for an incoming frame that belongs
/// to the discovery protocol (property / schema / shadow). Returns `None` for
/// regular monitoring data (handled by the value layer).
pub fn waiter_key_for_frame(can_class_byte: u8, device_addr: u32, data: &[u8]) -> Option<String> {
    match can_class_byte {
        can_class::PROPERTY_INFO => {
            if data.len() < 2 {
                return None;
            }
            if data[0] == 0x30 && data.len() >= 4 {
                let str_id = u16::from_le_bytes([data[1], data[2]]);
                Some(format!("str:{:06X}:{:04X}:{}", device_addr, str_id, data[3]))
            } else {
                Some(format!("p:{:06X}:{:02X}:{:02X}", device_addr, data[0], data[1]))
            }
        }
        can_class::SCHEMA_DATA => {
            if data.len() < 2 {
                return None;
            }
            match data[0] {
                0x03 if data.len() >= 4 => {
                    Some(format!("schema:{:06X}:03:{}:{}", device_addr, data[1], data[3]))
                }
                0x03 => None,
                op => Some(format!("schema:{:06X}:{:02X}:{}", device_addr, op, data[1])),
            }
        }
        can_class::MONITORING_DATA => {
            // Shadow response (high address bit set), or regular monitoring data.
            if device_addr & SHADOW_BIT != 0 {
                let real = device_addr & !SHADOW_BIT;
                if data.len() < 2 {
                    return None;
                }
                if data[0] == shadow_op::OPTION && data.len() >= 4 {
                    Some(format!("shadow:{:06X}:26:{}:{}", real, data[1], data[3]))
                } else {
                    Some(format!("shadow:{:06X}:{:02X}:{}", real, data[0], data[1]))
                }
            } else {
                None
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_decodes_packed_integer() {
        // 843000.0 = 2026*416 + 5*32 + 24  ->  24/05/2026
        let raw = 843000.0f32.to_le_bytes();
        assert_eq!(decode_value(&raw, VisualizationType::Date), Value::Date(Date { day: 24, mon: 5, year: 2026 }));
    }

    #[test]
    fn time_decodes_total_seconds() {
        // 54496 s -> 0 days, 15:08:16
        let raw = 54496.0f32.to_le_bytes();
        assert_eq!(decode_value(&raw, VisualizationType::Time), Value::Time(Time { sec: 16, min: 8, hour: 15, days: 0 }));
    }

    #[test]
    fn float_and_bool() {
        assert_eq!(decode_value(&9.0f32.to_le_bytes(), VisualizationType::Float), Value::Float(9.0));
        assert_eq!(decode_value(&[1, 0, 0, 0], VisualizationType::CheckBox), Value::Boolean(true));
    }
}
