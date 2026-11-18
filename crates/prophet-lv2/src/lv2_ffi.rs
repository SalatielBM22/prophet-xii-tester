// SPDX-License-Identifier: GPL-3.0-only
//! Minimal C-ABI LV2 specification bindings verified directly against sysroot headers
//! (`/usr/include/lv2/...`).

#![allow(non_camel_case_types, non_snake_case, dead_code)]

use core::ffi::{c_char, c_double, c_float, c_int, c_void, CStr};

pub const PLUGIN_URI: &CStr = c"http://prophet-xii.org/plugins/prophet-xii";
pub const UI_URI: &CStr = c"http://prophet-xii.org/plugins/prophet-xii#ui";
pub const STATE_PRESET_V2_URI: &CStr = c"http://prophet-xii.org/plugins/prophet-xii#presetV2";

pub const PLUGIN_URI_STR: &str = "http://prophet-xii.org/plugins/prophet-xii";
pub const UI_URI_STR: &str = "http://prophet-xii.org/plugins/prophet-xii#ui";
pub const STATE_PRESET_V2_URI_STR: &str = "http://prophet-xii.org/plugins/prophet-xii#presetV2";

// Core LV2 URIs
pub const LV2_URID_MAP_URI: &CStr = c"http://lv2plug.in/ns/ext/urid#map";
pub const LV2_URID_UNMAP_URI: &CStr = c"http://lv2plug.in/ns/ext/urid#unmap";

pub const LV2_ATOM_SEQUENCE_URI: &CStr = c"http://lv2plug.in/ns/ext/atom#Sequence";
pub const LV2_ATOM_OBJECT_URI: &CStr = c"http://lv2plug.in/ns/ext/atom#Object";
pub const LV2_ATOM_BLANK_URI: &CStr = c"http://lv2plug.in/ns/ext/atom#Blank";
pub const LV2_ATOM_FLOAT_URI: &CStr = c"http://lv2plug.in/ns/ext/atom#Float";
pub const LV2_ATOM_DOUBLE_URI: &CStr = c"http://lv2plug.in/ns/ext/atom#Double";
pub const LV2_ATOM_INT_URI: &CStr = c"http://lv2plug.in/ns/ext/atom#Int";
pub const LV2_ATOM_LONG_URI: &CStr = c"http://lv2plug.in/ns/ext/atom#Long";
pub const LV2_ATOM_STRING_URI: &CStr = c"http://lv2plug.in/ns/ext/atom#String";
pub const LV2_ATOM_CHUNK_URI: &CStr = c"http://lv2plug.in/ns/ext/atom#Chunk";
pub const LV2_ATOM_URID_URI: &CStr = c"http://lv2plug.in/ns/ext/atom#URID";

pub const LV2_MIDI_EVENT_URI: &CStr = c"http://lv2plug.in/ns/ext/midi#MidiEvent";

pub const LV2_TIME_POSITION_URI: &CStr = c"http://lv2plug.in/ns/ext/time#Position";
pub const LV2_TIME_BEATS_PER_MINUTE_URI: &CStr = c"http://lv2plug.in/ns/ext/time#beatsPerMinute";

pub const LV2_PATCH_SET_URI: &CStr = c"http://lv2plug.in/ns/ext/patch#Set";
pub const LV2_PATCH_PROPERTY_URI: &CStr = c"http://lv2plug.in/ns/ext/patch#property";
pub const LV2_PATCH_VALUE_URI: &CStr = c"http://lv2plug.in/ns/ext/patch#value";

pub const LV2_BUF_SIZE_MAX_BLOCK_LENGTH_URI: &CStr =
    c"http://lv2plug.in/ns/ext/buf-size#maxBlockLength";

pub const LV2_OPTIONS_OPTIONS_URI: &CStr = c"http://lv2plug.in/ns/ext/options#options";
pub const LV2_OPTIONS_INTERFACE_URI: &CStr = c"http://lv2plug.in/ns/ext/options#interface";

pub const LV2_WORKER_SCHEDULE_URI: &CStr = c"http://lv2plug.in/ns/ext/worker#schedule";
pub const LV2_WORKER_INTERFACE_URI: &CStr = c"http://lv2plug.in/ns/ext/worker#interface";

pub const LV2_STATE_INTERFACE_URI: &CStr = c"http://lv2plug.in/ns/ext/state#interface";

pub const LV2_UI_PARENT_URI: &CStr = c"http://lv2plug.in/ns/extensions/ui#parent";
pub const LV2_UI_RESIZE_URI: &CStr = c"http://lv2plug.in/ns/extensions/ui#resize";
pub const LV2_UI_IDLE_INTERFACE_URI: &CStr = c"http://lv2plug.in/ns/extensions/ui#idleInterface";
pub const LV2_UI_SHOW_INTERFACE_URI: &CStr = c"http://lv2plug.in/ns/extensions/ui#showInterface";
pub const LV2_INSTANCE_ACCESS_URI: &CStr = c"http://lv2plug.in/ns/ext/instance-access";

pub type LV2_Handle = *mut c_void;
pub type LV2_URID = u32;
pub type LV2_URID_Map_Handle = *mut c_void;
pub type LV2_URID_Unmap_Handle = *mut c_void;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct LV2_Feature {
    pub URI: *const c_char,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct LV2_Descriptor {
    pub URI: *const c_char,
    pub instantiate: Option<
        unsafe extern "C" fn(
            descriptor: *const LV2_Descriptor,
            sample_rate: c_double,
            bundle_path: *const c_char,
            features: *const *const LV2_Feature,
        ) -> LV2_Handle,
    >,
    pub connect_port:
        Option<unsafe extern "C" fn(instance: LV2_Handle, port: u32, data_location: *mut c_void)>,
    pub activate: Option<unsafe extern "C" fn(instance: LV2_Handle)>,
    pub run: Option<unsafe extern "C" fn(instance: LV2_Handle, sample_count: u32)>,
    pub deactivate: Option<unsafe extern "C" fn(instance: LV2_Handle)>,
    pub cleanup: Option<unsafe extern "C" fn(instance: LV2_Handle)>,
    pub extension_data: Option<unsafe extern "C" fn(uri: *const c_char) -> *const c_void>,
}

unsafe impl Sync for LV2_Descriptor {}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct LV2_URID_Map {
    pub handle: LV2_URID_Map_Handle,
    pub map: Option<unsafe extern "C" fn(handle: LV2_URID_Map_Handle, uri: *const c_char) -> LV2_URID>,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct LV2_URID_Unmap {
    pub handle: LV2_URID_Unmap_Handle,
    pub unmap:
        Option<unsafe extern "C" fn(handle: LV2_URID_Unmap_Handle, urid: LV2_URID) -> *const c_char>,
}

// Atom structs matching lv2/atom/atom.h

#[repr(C)]
#[derive(Copy, Clone)]
pub struct LV2_Atom {
    pub size: u32,
    pub atom_type: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct LV2_Atom_Int {
    pub atom: LV2_Atom,
    pub body: i32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct LV2_Atom_Long {
    pub atom: LV2_Atom,
    pub body: i64,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct LV2_Atom_Float {
    pub atom: LV2_Atom,
    pub body: c_float,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct LV2_Atom_Double {
    pub atom: LV2_Atom,
    pub body: c_double,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct LV2_Atom_URID {
    pub atom: LV2_Atom,
    pub body: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct LV2_Atom_Property_Body {
    pub key: u32,
    pub context: u32,
    pub value: LV2_Atom,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct LV2_Atom_Object_Body {
    pub id: u32,
    pub otype: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct LV2_Atom_Object {
    pub atom: LV2_Atom,
    pub body: LV2_Atom_Object_Body,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub union LV2_Atom_Event_Time {
    pub frames: i64,
    pub beats: c_double,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct LV2_Atom_Event {
    pub time: LV2_Atom_Event_Time,
    pub body: LV2_Atom,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct LV2_Atom_Sequence_Body {
    pub unit: u32,
    pub pad: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct LV2_Atom_Sequence {
    pub atom: LV2_Atom,
    pub body: LV2_Atom_Sequence_Body,
}

// Worker extension matching lv2/worker/worker.h

pub type LV2_Worker_Status = c_int;
pub const LV2_WORKER_SUCCESS: LV2_Worker_Status = 0;
pub const LV2_WORKER_ERR_UNKNOWN: LV2_Worker_Status = 1;
pub const LV2_WORKER_ERR_NO_SPACE: LV2_Worker_Status = 2;

pub type LV2_Worker_Respond_Handle = *mut c_void;
pub type LV2_Worker_Schedule_Handle = *mut c_void;

pub type LV2_Worker_Respond_Function = Option<
    unsafe extern "C" fn(
        handle: LV2_Worker_Respond_Handle,
        size: u32,
        data: *const c_void,
    ) -> LV2_Worker_Status,
>;

#[repr(C)]
pub struct LV2_Worker_Interface {
    pub work: Option<
        unsafe extern "C" fn(
            instance: LV2_Handle,
            respond: LV2_Worker_Respond_Function,
            handle: LV2_Worker_Respond_Handle,
            size: u32,
            data: *const c_void,
        ) -> LV2_Worker_Status,
    >,
    pub work_response: Option<
        unsafe extern "C" fn(
            instance: LV2_Handle,
            size: u32,
            body: *const c_void,
        ) -> LV2_Worker_Status,
    >,
    pub end_run: Option<unsafe extern "C" fn(instance: LV2_Handle) -> LV2_Worker_Status>,
}

unsafe impl Sync for LV2_Worker_Interface {}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct LV2_Worker_Schedule {
    pub handle: LV2_Worker_Schedule_Handle,
    pub schedule_work: Option<
        unsafe extern "C" fn(
            handle: LV2_Worker_Schedule_Handle,
            size: u32,
            data: *const c_void,
        ) -> LV2_Worker_Status,
    >,
}

unsafe impl Send for LV2_Worker_Schedule {}

// Options extension matching lv2/options/options.h

pub type LV2_Options_Context = c_int;
pub const LV2_OPTIONS_INSTANCE: LV2_Options_Context = 0;
pub const LV2_OPTIONS_SUCCESS: u32 = 0;
pub const LV2_OPTIONS_ERR_BAD_KEY: u32 = 1 << 2;
pub const LV2_OPTIONS_ERR_BAD_VALUE: u32 = 1 << 3;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct LV2_Options_Option {
    pub context: LV2_Options_Context,
    pub subject: u32,
    pub key: LV2_URID,
    pub size: u32,
    pub option_type: LV2_URID,
    pub value: *const c_void,
}

#[repr(C)]
pub struct LV2_Options_Interface {
    pub get: Option<
        unsafe extern "C" fn(instance: LV2_Handle, options: *mut LV2_Options_Option) -> u32,
    >,
    pub set: Option<
        unsafe extern "C" fn(instance: LV2_Handle, options: *const LV2_Options_Option) -> u32,
    >,
}

unsafe impl Sync for LV2_Options_Interface {}

// State extension matching lv2/state/state.h

pub type LV2_State_Handle = *mut c_void;
pub type LV2_State_Status = c_int;

pub const LV2_STATE_IS_POD: u32 = 1 << 0;
pub const LV2_STATE_IS_PORTABLE: u32 = 1 << 1;

pub const LV2_STATE_SUCCESS: LV2_State_Status = 0;
pub const LV2_STATE_ERR_UNKNOWN: LV2_State_Status = 1;
pub const LV2_STATE_ERR_BAD_TYPE: LV2_State_Status = 2;

pub type LV2_State_Store_Function = Option<
    unsafe extern "C" fn(
        handle: LV2_State_Handle,
        key: u32,
        value: *const c_void,
        size: usize,
        val_type: u32,
        flags: u32,
    ) -> LV2_State_Status,
>;

pub type LV2_State_Retrieve_Function = Option<
    unsafe extern "C" fn(
        handle: LV2_State_Handle,
        key: u32,
        size: *mut usize,
        val_type: *mut u32,
        flags: *mut u32,
    ) -> *const c_void,
>;

#[repr(C)]
pub struct LV2_State_Interface {
    pub save: Option<
        unsafe extern "C" fn(
            instance: LV2_Handle,
            store: LV2_State_Store_Function,
            handle: LV2_State_Handle,
            flags: u32,
            features: *const *const LV2_Feature,
        ) -> LV2_State_Status,
    >,
    pub restore: Option<
        unsafe extern "C" fn(
            instance: LV2_Handle,
            retrieve: LV2_State_Retrieve_Function,
            handle: LV2_State_Handle,
            flags: u32,
            features: *const *const LV2_Feature,
        ) -> LV2_State_Status,
    >,
}

unsafe impl Sync for LV2_State_Interface {}

// UI extension matching lv2/ui/ui.h

pub type LV2UI_Widget = *mut c_void;
pub type LV2UI_Handle = *mut c_void;
pub type LV2UI_Controller = *mut c_void;
pub type LV2UI_Feature_Handle = *mut c_void;

pub type LV2UI_Write_Function = Option<
    unsafe extern "C" fn(
        controller: LV2UI_Controller,
        port_index: u32,
        buffer_size: u32,
        port_protocol: u32,
        buffer: *const c_void,
    ),
>;

#[repr(C)]
pub struct LV2UI_Descriptor {
    pub URI: *const c_char,
    pub instantiate: Option<
        unsafe extern "C" fn(
            descriptor: *const LV2UI_Descriptor,
            plugin_uri: *const c_char,
            bundle_path: *const c_char,
            write_function: LV2UI_Write_Function,
            controller: LV2UI_Controller,
            widget: *mut LV2UI_Widget,
            features: *const *const LV2_Feature,
        ) -> LV2UI_Handle,
    >,
    pub cleanup: Option<unsafe extern "C" fn(ui: LV2UI_Handle)>,
    pub port_event: Option<
        unsafe extern "C" fn(
            ui: LV2UI_Handle,
            port_index: u32,
            buffer_size: u32,
            format: u32,
            buffer: *const c_void,
        ),
    >,
    pub extension_data: Option<unsafe extern "C" fn(uri: *const c_char) -> *const c_void>,
}

unsafe impl Sync for LV2UI_Descriptor {}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct LV2UI_Resize {
    pub handle: LV2UI_Feature_Handle,
    pub ui_resize:
        Option<unsafe extern "C" fn(handle: LV2UI_Feature_Handle, width: c_int, height: c_int) -> c_int>,
}

unsafe impl Send for LV2UI_Resize {}
unsafe impl Sync for LV2UI_Resize {}

#[repr(C)]
pub struct LV2UI_Idle_Interface {
    pub idle: Option<unsafe extern "C" fn(ui: LV2UI_Handle) -> c_int>,
}

unsafe impl Sync for LV2UI_Idle_Interface {}

#[repr(C)]
pub struct LV2UI_Show_Interface {
    pub show: Option<unsafe extern "C" fn(ui: LV2UI_Handle) -> c_int>,
    pub hide: Option<unsafe extern "C" fn(ui: LV2UI_Handle) -> c_int>,
}

unsafe impl Sync for LV2UI_Show_Interface {}

#[inline]
pub fn atom_pad_size(size: u32) -> usize {
    ((size as usize) + 7) & !7
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{align_of, size_of};

    #[test]
    fn lv2_abi_struct_sizes() {
        assert_eq!(size_of::<LV2_Atom>(), 8);
        assert_eq!(size_of::<LV2_Atom_Sequence_Body>(), 8);
        assert_eq!(size_of::<LV2_Atom_Sequence>(), 16);
        assert_eq!(size_of::<LV2_Atom_Event>(), 16);
        assert_eq!(size_of::<LV2_Atom_Object_Body>(), 8);
        assert_eq!(size_of::<LV2_Atom_Object>(), 16);
        assert_eq!(size_of::<LV2_Atom_Property_Body>(), 16);
        assert_eq!(align_of::<LV2_Atom>(), 4);
        assert_eq!(align_of::<LV2_Atom_Event>(), 8);
    }
}
