//! C ABI for the `masterbus` crate.
//!
//! **Single-threaded contract**: an `MbBus` handle (and any `MbValue` obtained
//! through it) must only be used from one thread. The header `masterbus.h` is
//! generated from this crate via cbindgen (see `build.rs`).
//!
//! Ownership rules:
//! - `mb_open_*` returns an `MbBus*`; free it with [`mb_close`].
//! - Functions returning `char*` allocate; free with [`mb_free_str`].
//! - Functions returning `MbValue*` allocate; free with [`mb_free_value`].
//! - Array out-params (`mb_devices`, `mb_group_fields`) allocate; free with the
//!   matching `mb_free_*` function.
//! - `NULL` is returned on any error (unknown device/field, not ready, etc.).

// This crate is an inherently-unsafe C boundary: every entry point dereferences
// pointers supplied by the C caller. The contract is documented above rather
// than encoded as `unsafe fn` (which would not change the generated C ABI).
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::PathBuf;
use std::ptr;

use masterbus::{Config, MasterBus, Menu, Value};

/// Opaque connection handle.
pub struct MbBus {
    inner: MasterBus,
}

/// Opaque decoded value.
pub struct MbValue {
    inner: Value,
}

/// Discriminant of an [`MbValue`].
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MbValueType {
    Float = 0,
    Date = 1,
    Time = 2,
    Boolean = 3,
    List = 4,
    Text = 5,
    DeviceRef = 6,
    Eventable = 7,
    Invalid = 8,
}

/// Operational status of a device.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MbStatus {
    Offline = 0,
    Sleeping = 1,
    On = 2,
    OnWarning = 3,
    OffFault = 4,
    OffError = 5,
    Updating = 6,
    Unknown = 7,
}

/// Calendar date (mirrors `masterbus::Date`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MbDate {
    pub day: i32,
    pub mon: i32,
    pub year: i32,
}

/// Clock / duration (mirrors `masterbus::Time`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MbTime {
    pub sec: i32,
    pub min: i32,
    pub hour: i32,
    pub days: u32,
}

// ---- helpers --------------------------------------------------------------

fn bus_ref<'a>(bus: *const MbBus) -> Option<&'a MasterBus> {
    unsafe { bus.as_ref() }.map(|b| &b.inner)
}

fn value_ref<'a>(v: *const MbValue) -> Option<&'a Value> {
    unsafe { v.as_ref() }.map(|v| &v.inner)
}

/// Move a `String` into a freshly-allocated C string (NUL-terminated).
/// Returns NULL if the string contains an interior NUL.
fn to_cstr(s: String) -> *mut c_char {
    match CString::new(s) {
        Ok(c) => c.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

fn opt_str<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(p) }.to_str().ok()
}

fn alloc_value(v: Value) -> *mut MbValue {
    Box::into_raw(Box::new(MbValue { inner: v }))
}

fn alloc_slice<T>(v: Vec<T>) -> (*mut T, i32) {
    let boxed = v.into_boxed_slice();
    let len = boxed.len() as i32;
    let ptr = Box::into_raw(boxed) as *mut T;
    (ptr, len)
}

// ---- connection -----------------------------------------------------------

/// Open a SocketCAN connection (Linux only; returns NULL elsewhere).
///
/// `cache_dir` may be NULL (memory-only) or a directory for the schema cache.
#[unsafe(no_mangle)]
pub extern "C" fn mb_open_socketcan(iface: *const c_char, cache_dir: *const c_char) -> *mut MbBus {
    let Some(iface) = opt_str(iface) else {
        return ptr::null_mut();
    };
    let config = Config {
        cache_path: opt_str(cache_dir).map(PathBuf::from),
        ..Default::default()
    };
    #[cfg(target_os = "linux")]
    {
        match MasterBus::socketcan(iface, config) {
            Ok(b) => Box::into_raw(Box::new(MbBus { inner: b })),
            Err(_) => ptr::null_mut(),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (iface, config);
        ptr::null_mut()
    }
}

/// Close and free a bus handle.
#[unsafe(no_mangle)]
pub extern "C" fn mb_close(bus: *mut MbBus) {
    if !bus.is_null() {
        drop(unsafe { Box::from_raw(bus) });
    }
}

// ---- devices --------------------------------------------------------------

/// Fill `*out_ids` with the ids of every device on the bus and return the count
/// (waits up to ~2 s for the broadcast list to fill). Returns -1 on error.
/// Free the array with [`mb_free_ids`].
#[unsafe(no_mangle)]
pub extern "C" fn mb_devices(bus: *mut MbBus, out_ids: *mut *mut u32) -> i32 {
    let Some(bus) = bus_ref(bus) else { return -1 };
    if out_ids.is_null() {
        return -1;
    }
    let ids: Vec<u32> = bus.devices_all().iter().map(|d| d.id()).collect();
    let (ptr, len) = alloc_slice(ids);
    unsafe { *out_ids = ptr };
    len
}

/// Free an id array returned by [`mb_devices`].
#[unsafe(no_mangle)]
pub extern "C" fn mb_free_ids(ids: *mut u32, len: i32) {
    if !ids.is_null() && len > 0 {
        let s = ptr::slice_from_raw_parts_mut(ids, len as usize);
        drop(unsafe { Box::from_raw(s) });
    }
}

/// Device product name (NULL on error). Free with [`mb_free_str`].
#[unsafe(no_mangle)]
pub extern "C" fn mb_device_name(bus: *mut MbBus, id: u32) -> *mut c_char {
    match bus_ref(bus).and_then(|b| b.device(id).name().ok()) {
        Some(s) => to_cstr(s),
        None => ptr::null_mut(),
    }
}

/// Device article number (NULL on error). Free with [`mb_free_str`].
#[unsafe(no_mangle)]
pub extern "C" fn mb_device_article(bus: *mut MbBus, id: u32) -> *mut c_char {
    match bus_ref(bus).and_then(|b| b.device(id).article_number().ok()) {
        Some(s) => to_cstr(s),
        None => ptr::null_mut(),
    }
}

/// Device serial number (NULL on error). Free with [`mb_free_str`].
#[unsafe(no_mangle)]
pub extern "C" fn mb_device_serial(bus: *mut MbBus, id: u32) -> *mut c_char {
    match bus_ref(bus).and_then(|b| b.device(id).serial_number().ok()) {
        Some(s) => to_cstr(s),
        None => ptr::null_mut(),
    }
}

/// Device revision code (NULL on error). Free with [`mb_free_str`].
#[unsafe(no_mangle)]
pub extern "C" fn mb_device_revision(bus: *mut MbBus, id: u32) -> *mut c_char {
    match bus_ref(bus).and_then(|b| b.device(id).revision_code().ok()) {
        Some(s) => to_cstr(s),
        None => ptr::null_mut(),
    }
}

/// Device firmware version (NULL on error). Free with [`mb_free_str`].
#[unsafe(no_mangle)]
pub extern "C" fn mb_device_firmware(bus: *mut MbBus, id: u32) -> *mut c_char {
    match bus_ref(bus).and_then(|b| b.device(id).firmware_version().ok()) {
        Some(s) => to_cstr(s),
        None => ptr::null_mut(),
    }
}

/// Device operational status.
#[unsafe(no_mangle)]
pub extern "C" fn mb_device_status(bus: *mut MbBus, id: u32) -> MbStatus {
    use masterbus::DeviceStatus as S;
    let Some(bus) = bus_ref(bus) else {
        return MbStatus::Unknown;
    };
    match bus.device(id).status() {
        S::Offline => MbStatus::Offline,
        S::Sleeping => MbStatus::Sleeping,
        S::On => MbStatus::On,
        S::OnWarning => MbStatus::OnWarning,
        S::OffFault => MbStatus::OffFault,
        S::OffError => MbStatus::OffError,
        S::Updating => MbStatus::Updating,
        S::Unknown => MbStatus::Unknown,
    }
}

// ---- groups & fields (monitoring menu) ------------------------------------

/// Number of monitoring groups on a device (-1 on error).
#[unsafe(no_mangle)]
pub extern "C" fn mb_group_count(bus: *mut MbBus, id: u32) -> i32 {
    match bus_ref(bus).and_then(|b| b.device(id).tab(Menu::Monitoring).ok()) {
        Some(groups) => groups.len() as i32,
        None => -1,
    }
}

/// Name of the `group_index`-th monitoring group (NULL on error).
#[unsafe(no_mangle)]
pub extern "C" fn mb_group_name(bus: *mut MbBus, id: u32, group_index: i32) -> *mut c_char {
    let Some(bus) = bus_ref(bus) else {
        return ptr::null_mut();
    };
    let Ok(groups) = bus.device(id).tab(Menu::Monitoring) else {
        return ptr::null_mut();
    };
    match groups.get(group_index as usize).and_then(|g| g.name().ok()) {
        Some(s) => to_cstr(s),
        None => ptr::null_mut(),
    }
}

/// Fill `*out_fields` with the field indices of the `group_index`-th monitoring
/// group; returns the count (-1 on error). Free with [`mb_free_fields`].
#[unsafe(no_mangle)]
pub extern "C" fn mb_group_fields(
    bus: *mut MbBus,
    id: u32,
    group_index: i32,
    out_fields: *mut *mut i32,
) -> i32 {
    let Some(bus) = bus_ref(bus) else { return -1 };
    if out_fields.is_null() {
        return -1;
    }
    let Ok(groups) = bus.device(id).tab(Menu::Monitoring) else {
        return -1;
    };
    let Some(group) = groups.get(group_index as usize) else {
        return -1;
    };
    let Ok(fields) = group.fields() else {
        return -1;
    };
    // The C ABI uses `i32` for field ids; channel-aware `FieldId` (u16)
    // fits losslessly.
    let idx: Vec<i32> = fields.iter().map(|f| f.index() as i32).collect();
    let (ptr, len) = alloc_slice(idx);
    unsafe { *out_fields = ptr };
    len
}

/// Free a field-index array returned by [`mb_group_fields`].
#[unsafe(no_mangle)]
pub extern "C" fn mb_free_fields(fields: *mut i32, len: i32) {
    if !fields.is_null() && len > 0 {
        let s = ptr::slice_from_raw_parts_mut(fields, len as usize);
        drop(unsafe { Box::from_raw(s) });
    }
}

/// Field name (NULL on error). Free with [`mb_free_str`].
#[unsafe(no_mangle)]
pub extern "C" fn mb_field_name(bus: *mut MbBus, id: u32, field: i32) -> *mut c_char {
    match bus_ref(bus).and_then(|b| b.device(id).field(field as u16).name().ok()) {
        Some(s) => to_cstr(s),
        None => ptr::null_mut(),
    }
}

/// Field unit, possibly empty (NULL on error). Free with [`mb_free_str`].
#[unsafe(no_mangle)]
pub extern "C" fn mb_field_unit(bus: *mut MbBus, id: u32, field: i32) -> *mut c_char {
    match bus_ref(bus).and_then(|b| b.device(id).field(field as u16).unit().ok()) {
        Some(s) => to_cstr(s),
        None => ptr::null_mut(),
    }
}

/// Whether a field is currently writable (false on error).
#[unsafe(no_mangle)]
pub extern "C" fn mb_field_writable(bus: *mut MbBus, id: u32, field: i32) -> bool {
    bus_ref(bus)
        .and_then(|b| b.device(id).field(field as u16).is_writable().ok())
        .unwrap_or(false)
}

// ---- reads & writes -------------------------------------------------------

/// Read a field's current value (NULL on error). Free with [`mb_free_value`].
#[unsafe(no_mangle)]
pub extern "C" fn mb_field_value(bus: *mut MbBus, id: u32, field: i32) -> *mut MbValue {
    match bus_ref(bus).and_then(|b| b.device(id).field(field as u16).value().ok()) {
        Some(v) => alloc_value(v),
        None => ptr::null_mut(),
    }
}

/// Write a boolean and return the value observed afterwards (NULL on error /
/// rejected write). Free with [`mb_free_value`].
#[unsafe(no_mangle)]
pub extern "C" fn mb_set_bool(bus: *mut MbBus, id: u32, field: i32, value: bool) -> *mut MbValue {
    match bus_ref(bus).and_then(|b| {
        b.device(id)
            .field(field as u16)
            .set(Value::Boolean(value))
            .ok()
    }) {
        Some(v) => alloc_value(v),
        None => ptr::null_mut(),
    }
}

/// Write a float and return the value observed afterwards (NULL on error /
/// rejected write). Free with [`mb_free_value`].
#[unsafe(no_mangle)]
pub extern "C" fn mb_set_float(bus: *mut MbBus, id: u32, field: i32, value: f32) -> *mut MbValue {
    match bus_ref(bus).and_then(|b| {
        b.device(id)
            .field(field as u16)
            .set(Value::Float(value))
            .ok()
    }) {
        Some(v) => alloc_value(v),
        None => ptr::null_mut(),
    }
}

// ---- value accessors ------------------------------------------------------

/// Discriminant of a value.
#[unsafe(no_mangle)]
pub extern "C" fn mb_value_type(v: *const MbValue) -> MbValueType {
    match value_ref(v) {
        Some(Value::Float(_)) => MbValueType::Float,
        Some(Value::Date(_)) => MbValueType::Date,
        Some(Value::Time(_)) => MbValueType::Time,
        Some(Value::Boolean(_)) => MbValueType::Boolean,
        Some(Value::List { .. }) => MbValueType::List,
        Some(Value::Text { .. }) => MbValueType::Text,
        Some(Value::DeviceRef { .. }) => MbValueType::DeviceRef,
        Some(Value::Eventable { .. }) => MbValueType::Eventable,
        _ => MbValueType::Invalid,
    }
}

/// Float payload (0.0 if not a float).
#[unsafe(no_mangle)]
pub extern "C" fn mb_value_float(v: *const MbValue) -> f32 {
    match value_ref(v) {
        Some(Value::Float(f)) => *f,
        _ => 0.0,
    }
}

/// Boolean payload (false if not a boolean).
#[unsafe(no_mangle)]
pub extern "C" fn mb_value_bool(v: *const MbValue) -> bool {
    matches!(value_ref(v), Some(Value::Boolean(true)))
}

/// Date payload (day=-1 if not a date).
#[unsafe(no_mangle)]
pub extern "C" fn mb_value_date(v: *const MbValue) -> MbDate {
    match value_ref(v) {
        Some(Value::Date(d)) => MbDate {
            day: d.day,
            mon: d.mon,
            year: d.year,
        },
        _ => MbDate {
            day: -1,
            mon: -1,
            year: -1,
        },
    }
}

/// Time payload (sec=-1 if not a time).
#[unsafe(no_mangle)]
pub extern "C" fn mb_value_time(v: *const MbValue) -> MbTime {
    match value_ref(v) {
        Some(Value::Time(t)) => MbTime {
            sec: t.sec,
            min: t.min,
            hour: t.hour,
            days: t.days,
        },
        _ => MbTime {
            sec: -1,
            min: -1,
            hour: -1,
            days: 0,
        },
    }
}

/// Text payload (NULL if not text). Free with [`mb_free_str`].
#[unsafe(no_mangle)]
pub extern "C" fn mb_value_text(v: *const MbValue) -> *mut c_char {
    match value_ref(v) {
        Some(Value::Text { text, .. }) => to_cstr(text.clone()),
        _ => ptr::null_mut(),
    }
}

/// Selected index for list/enum/device-ref/eventable values (-1 otherwise).
#[unsafe(no_mangle)]
pub extern "C" fn mb_value_list_index(v: *const MbValue) -> i32 {
    match value_ref(v) {
        Some(Value::List { index, .. })
        | Some(Value::Eventable { index, .. })
        | Some(Value::DeviceRef { index, .. }) => *index,
        _ => -1,
    }
}

/// Number of options/entries for list/eventable/device-ref values (0 otherwise).
#[unsafe(no_mangle)]
pub extern "C" fn mb_value_list_size(v: *const MbValue) -> i32 {
    match value_ref(v) {
        Some(Value::List { options, .. }) => options.len() as i32,
        Some(Value::Eventable { labels, .. }) => labels.len() as i32,
        Some(Value::DeviceRef { device_ids, .. }) => device_ids.len() as i32,
        _ => 0,
    }
}

/// Label of the `index`-th option of a list/eventable value (NULL otherwise).
/// Free with [`mb_free_str`].
#[unsafe(no_mangle)]
pub extern "C" fn mb_value_list_label(v: *const MbValue, index: i32) -> *mut c_char {
    let i = index as usize;
    let label = match value_ref(v) {
        Some(Value::List { options, .. }) => options.get(i),
        Some(Value::Eventable { labels, .. }) => labels.get(i),
        _ => None,
    };
    match label {
        Some(s) => to_cstr(s.clone()),
        None => ptr::null_mut(),
    }
}

/// Referenced device id at `index` for a device-ref value (0 otherwise).
#[unsafe(no_mangle)]
pub extern "C" fn mb_value_device_id(v: *const MbValue, index: i32) -> u32 {
    match value_ref(v) {
        Some(Value::DeviceRef { device_ids, .. }) => {
            device_ids.get(index as usize).copied().unwrap_or(0)
        }
        _ => 0,
    }
}

/// Free a value returned by `mb_field_value` / `mb_set_*`.
#[unsafe(no_mangle)]
pub extern "C" fn mb_free_value(v: *mut MbValue) {
    if !v.is_null() {
        drop(unsafe { Box::from_raw(v) });
    }
}

/// Free a string returned by any `mb_*` function.
#[unsafe(no_mangle)]
pub extern "C" fn mb_free_str(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use masterbus::{Date, Time};

    /// Read back a `char*` this crate allocated, then free it.
    fn take(p: *mut c_char) -> Option<String> {
        if p.is_null() {
            return None;
        }
        let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
        mb_free_str(p);
        Some(s)
    }

    /// Wrap a value the way the bus entry points do, so the accessors see
    /// exactly what a C caller would hold.
    fn value(v: Value) -> *mut MbValue {
        alloc_value(v)
    }

    /// NULL is the documented error return, so every accessor has to survive
    /// being handed one — this is the crate's main safety contract.
    #[test]
    fn every_accessor_survives_a_null_value() {
        let v: *const MbValue = ptr::null();
        assert!(mb_value_type(v) == MbValueType::Invalid);
        assert_eq!(mb_value_float(v), 0.0);
        assert!(!mb_value_bool(v));
        assert_eq!(mb_value_date(v).day, -1);
        assert_eq!(mb_value_time(v).sec, -1);
        assert!(mb_value_text(v).is_null());
        assert_eq!(mb_value_list_index(v), -1);
        assert_eq!(mb_value_list_size(v), 0);
        assert!(mb_value_list_label(v, 0).is_null());
        assert_eq!(mb_value_device_id(v, 0), 0);
    }

    /// The freeing functions are no-ops on NULL and on an empty array, so a C
    /// caller can free unconditionally after an error.
    #[test]
    fn the_free_functions_tolerate_null() {
        mb_free_str(ptr::null_mut());
        mb_free_value(ptr::null_mut());
        mb_free_ids(ptr::null_mut(), 0);
        mb_free_fields(ptr::null_mut(), 0);
        mb_free_ids(ptr::null_mut(), 5);
        mb_free_fields(ptr::null_mut(), 5);
        mb_close(ptr::null_mut());
    }

    #[test]
    fn each_value_kind_reports_its_own_discriminant() {
        let cases = [
            (Value::Float(1.0), MbValueType::Float),
            (
                Value::Date(Date {
                    day: 1,
                    mon: 2,
                    year: 2026,
                }),
                MbValueType::Date,
            ),
            (
                Value::Time(Time {
                    sec: 1,
                    min: 2,
                    hour: 3,
                    days: 4,
                }),
                MbValueType::Time,
            ),
            (Value::Boolean(true), MbValueType::Boolean),
            (
                Value::List {
                    index: 0,
                    options: vec![],
                },
                MbValueType::List,
            ),
            (
                Value::Text {
                    sid: 1,
                    text: String::new(),
                },
                MbValueType::Text,
            ),
            (
                Value::DeviceRef {
                    index: 0,
                    device_ids: vec![],
                },
                MbValueType::DeviceRef,
            ),
            (
                Value::Eventable {
                    index: 0,
                    labels: vec![],
                },
                MbValueType::Eventable,
            ),
            (Value::Invalid, MbValueType::Invalid),
        ];
        for (v, want) in cases {
            let p = value(v);
            assert!(mb_value_type(p) == want);
            mb_free_value(p);
        }
    }

    /// A payload accessor answers only for its own kind; asking a float for a
    /// date must give the documented sentinel, not a reinterpreted number.
    #[test]
    fn a_payload_accessor_answers_only_for_its_own_kind() {
        let p = value(Value::Float(12.5));
        assert_eq!(mb_value_float(p), 12.5);
        assert!(!mb_value_bool(p));
        assert_eq!(mb_value_date(p).day, -1);
        assert_eq!(mb_value_time(p).sec, -1);
        assert!(mb_value_text(p).is_null());
        assert_eq!(mb_value_list_index(p), -1);
        assert_eq!(mb_value_list_size(p), 0);
        mb_free_value(p);

        let p = value(Value::Boolean(true));
        assert!(mb_value_bool(p));
        assert_eq!(mb_value_float(p), 0.0);
        mb_free_value(p);

        // Boolean false and "not a boolean" are indistinguishable through the
        // C ABI — mb_value_type is how a caller tells them apart.
        let p = value(Value::Boolean(false));
        assert!(!mb_value_bool(p));
        assert!(mb_value_type(p) == MbValueType::Boolean);
        mb_free_value(p);
    }

    #[test]
    fn dates_and_times_carry_every_component() {
        let p = value(Value::Date(Date {
            day: 27,
            mon: 5,
            year: 2026,
        }));
        let d = mb_value_date(p);
        assert_eq!((d.day, d.mon, d.year), (27, 5, 2026));
        mb_free_value(p);

        let p = value(Value::Time(Time {
            sec: 9,
            min: 8,
            hour: 7,
            days: 6,
        }));
        let t = mb_value_time(p);
        assert_eq!((t.sec, t.min, t.hour, t.days), (9, 8, 7, 6));
        mb_free_value(p);
    }

    #[test]
    fn a_list_exposes_its_index_size_and_labels() {
        let p = value(Value::List {
            index: 1,
            options: vec!["Off".into(), "On".into()],
        });
        assert_eq!(mb_value_list_index(p), 1);
        assert_eq!(mb_value_list_size(p), 2);
        assert_eq!(take(mb_value_list_label(p, 0)).as_deref(), Some("Off"));
        assert_eq!(take(mb_value_list_label(p, 1)).as_deref(), Some("On"));
        // Out of range is NULL, not a panic across the ABI boundary.
        assert!(mb_value_list_label(p, 2).is_null());
        assert!(mb_value_list_label(p, -1).is_null());
        mb_free_value(p);
    }

    /// Eventable and device-ref values share the index accessor but each has
    /// its own notion of "entries".
    #[test]
    fn eventable_and_device_ref_values_share_the_index_accessor() {
        let p = value(Value::Eventable {
            index: 2,
            labels: vec!["Relay 1".into(), "Relay 2".into(), "Relay 3".into()],
        });
        assert_eq!(mb_value_list_index(p), 2);
        assert_eq!(mb_value_list_size(p), 3);
        assert_eq!(take(mb_value_list_label(p, 2)).as_deref(), Some("Relay 3"));
        assert_eq!(mb_value_device_id(p, 0), 0);
        mb_free_value(p);

        let p = value(Value::DeviceRef {
            index: 1,
            device_ids: vec![0x188EA2, 0x3A3B4B],
        });
        assert_eq!(mb_value_list_index(p), 1);
        assert_eq!(mb_value_list_size(p), 2);
        assert_eq!(mb_value_device_id(p, 1), 0x3A3B4B);
        // Out of range yields 0 — there is no device 0 on a MasterBus.
        assert_eq!(mb_value_device_id(p, 9), 0);
        // A device ref has ids, not labels.
        assert!(mb_value_list_label(p, 0).is_null());
        mb_free_value(p);
    }

    #[test]
    fn text_values_cross_the_boundary_as_c_strings() {
        let p = value(Value::Text {
            sid: 1,
            text: "NavigationCh".into(),
        });
        assert_eq!(take(mb_value_text(p)).as_deref(), Some("NavigationCh"));
        mb_free_value(p);
    }

    /// A Rust string may contain an interior NUL; a C string may not. The
    /// conversion refuses rather than truncating silently.
    #[test]
    fn a_string_with_an_interior_nul_becomes_null() {
        let p = value(Value::Text {
            sid: 1,
            text: "Nav\0Chg".into(),
        });
        assert!(mb_value_text(p).is_null());
        mb_free_value(p);

        // ...while an ordinary string converts and frees cleanly.
        assert_eq!(take(to_cstr("ok".into())).as_deref(), Some("ok"));
        assert!(to_cstr("bad\0".into()).is_null());
    }

    /// Every bus entry point returns its documented error value for a NULL
    /// handle, without dereferencing it.
    #[test]
    fn a_null_bus_fails_every_entry_point() {
        let bus: *mut MbBus = ptr::null_mut();
        let mut ids: *mut u32 = ptr::null_mut();
        let mut fields: *mut i32 = ptr::null_mut();

        assert_eq!(mb_devices(bus, &mut ids), -1);
        assert_eq!(mb_group_count(bus, 1), -1);
        assert_eq!(mb_group_fields(bus, 1, 0, &mut fields), -1);
        assert!(mb_group_name(bus, 1, 0).is_null());
        assert!(mb_device_name(bus, 1).is_null());
        assert!(mb_device_article(bus, 1).is_null());
        assert!(mb_device_serial(bus, 1).is_null());
        assert!(mb_device_revision(bus, 1).is_null());
        assert!(mb_device_firmware(bus, 1).is_null());
        assert!(mb_device_status(bus, 1) == MbStatus::Unknown);
        assert!(mb_field_name(bus, 1, 0).is_null());
        assert!(mb_field_unit(bus, 1, 0).is_null());
        assert!(!mb_field_writable(bus, 1, 0));
        assert!(mb_field_value(bus, 1, 0).is_null());
        assert!(mb_set_bool(bus, 1, 0, true).is_null());
        assert!(mb_set_float(bus, 1, 0, 1.0).is_null());
    }

    /// A NULL out-param is refused too: the array functions must not write
    /// through it before checking.
    #[test]
    fn a_null_out_parameter_is_refused() {
        let bus: *mut MbBus = ptr::null_mut();
        assert_eq!(mb_devices(bus, ptr::null_mut()), -1);
        assert_eq!(mb_group_fields(bus, 1, 0, ptr::null_mut()), -1);
    }

    #[test]
    fn opening_a_bus_rejects_a_null_interface_name() {
        assert!(mb_open_socketcan(ptr::null(), ptr::null()).is_null());
    }

    /// SocketCAN is Linux-only; elsewhere the entry point exists (so the ABI
    /// is identical everywhere) but always fails.
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn socketcan_is_unavailable_off_linux() {
        let iface = CString::new("can0").unwrap();
        assert!(mb_open_socketcan(iface.as_ptr(), ptr::null()).is_null());
    }

    #[test]
    fn borrowed_c_strings_are_optional_and_utf8_checked() {
        assert_eq!(opt_str(ptr::null()), None);
        let ok = CString::new("can0").unwrap();
        assert_eq!(opt_str(ok.as_ptr()), Some("can0"));
        // Invalid UTF-8 is treated as absent rather than lossily accepted.
        let bad = CString::new([0xFFu8, 0xFE]).unwrap();
        assert_eq!(opt_str(bad.as_ptr()), None);
    }

    /// The array helpers hand out a pointer plus a length that the matching
    /// free function accepts back.
    #[test]
    fn allocated_arrays_round_trip_through_their_free_function() {
        let (ids, len) = alloc_slice(vec![0x188EA2u32, 0x3A3B4B]);
        assert_eq!(len, 2);
        assert_eq!(
            unsafe { std::slice::from_raw_parts(ids, len as usize) },
            [0x188EA2, 0x3A3B4B]
        );
        mb_free_ids(ids, len);

        let (fields, len) = alloc_slice(vec![0x17i32, 0x118]);
        assert_eq!(len, 2);
        assert_eq!(unsafe { *fields.add(1) }, 0x118);
        mb_free_fields(fields, len);

        // An empty allocation is still a valid (dangling but non-null) pointer.
        let (empty, len) = alloc_slice(Vec::<u32>::new());
        assert_eq!(len, 0);
        mb_free_ids(empty, len);
    }
}
