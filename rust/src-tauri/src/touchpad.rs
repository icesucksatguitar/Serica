//! Four-finger touchpad swipes switch spaces.
//!
//! Windows keeps multi-finger gestures to itself (they never reach apps as
//! input messages), so this reads the precision touchpad's raw HID contact
//! reports through Raw Input on the main window and recognises a horizontal
//! four-finger swipe. Raw Input is only delivered while Serica is the
//! foreground app. Windows still runs its own four-finger gesture (switching
//! virtual desktops by default) unless the user sets it to "Nothing" under
//! Settings > Bluetooth & devices > Touchpad > Four-finger gestures.

use crate::app::{Shared, Shortcut};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use windows::Win32::Devices::HumanInterfaceDevice::*;
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::*;
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::WM_INPUT;

const PAGE_DIGITIZER: u16 = 0x0D;
const PAGE_GENERIC: u16 = 0x01;
const USAGE_TOUCHPAD: u16 = 0x05;
const USAGE_CONTACT_ID: u16 = 0x51;
const USAGE_CONTACT_COUNT: u16 = 0x54;
const USAGE_X: u16 = 0x30;
/// Swipe distance, as a share of the pad's width, that switches space.
const TRIGGER: f64 = 0.16;
/// A pause this long between reports ends the gesture (all fingers lifted).
const IDLE: Duration = Duration::from_millis(120);

static APP: OnceLock<Shared> = OnceLock::new();

struct Device {
    preparsed: Vec<u8>,
    /// Link collections carrying a contact's X, with the X logical range.
    contacts: Vec<(u16, i32, i32)>,
    /// Link collection of the contact-count usage.
    count_collection: Option<u16>,
}

#[derive(Default)]
struct Gesture {
    fingers: u32,
    xs: HashMap<u32, f64>,
    start: Option<f64>,
    fired: bool,
    last: Option<Instant>,
}

thread_local! {
    static DEVICES: RefCell<HashMap<isize, Option<Device>>> = RefCell::new(HashMap::new());
    static GESTURE: RefCell<Gesture> = RefCell::new(Gesture::default());
}

/// Registers for touchpad raw input on `hwnd` (the main window).
pub fn attach(app: &Shared, hwnd: HWND) {
    let _ = APP.set(app.clone());
    unsafe {
        let dev = RAWINPUTDEVICE { usUsagePage: PAGE_DIGITIZER, usUsage: USAGE_TOUCHPAD, dwFlags: RAWINPUTDEVICE_FLAGS(0), hwndTarget: hwnd };
        if let Err(e) = RegisterRawInputDevices(&[dev], std::mem::size_of::<RAWINPUTDEVICE>() as u32) {
            tracing::warn!("touchpad gestures unavailable: {e}");
            return;
        }
        let _ = SetWindowSubclass(hwnd, Some(subclass), 0x5E41CA, 0);
    }
}

unsafe extern "system" fn subclass(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
    if msg == WM_INPUT {
        let _ = std::panic::catch_unwind(|| on_input(HRAWINPUT(lparam.0 as _)));
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}

unsafe fn load_device(handle: HANDLE) -> Option<Device> {
    let mut size = 0u32;
    GetRawInputDeviceInfoW(Some(handle), RIDI_PREPARSEDDATA, None, &mut size);
    if size == 0 {
        return None;
    }
    let mut preparsed = vec![0u8; size as usize];
    if GetRawInputDeviceInfoW(Some(handle), RIDI_PREPARSEDDATA, Some(preparsed.as_mut_ptr() as _), &mut size) == u32::MAX {
        return None;
    }
    let pp = PHIDP_PREPARSED_DATA(preparsed.as_ptr() as _);
    let mut caps = HIDP_CAPS::default();
    if HidP_GetCaps(pp, &mut caps).is_err() {
        return None;
    }
    let mut n = caps.NumberInputValueCaps;
    let mut values = vec![HIDP_VALUE_CAPS::default(); n as usize];
    if HidP_GetValueCaps(HidP_Input, values.as_mut_ptr(), &mut n, pp).is_err() {
        return None;
    }
    let mut contacts = Vec::new();
    let mut count_collection = None;
    for v in &values[..n as usize] {
        let usage = if v.IsRange { v.Anonymous.Range.UsageMin } else { v.Anonymous.NotRange.Usage };
        if v.UsagePage == PAGE_GENERIC && usage == USAGE_X && v.LogicalMax > v.LogicalMin {
            contacts.push((v.LinkCollection, v.LogicalMin, v.LogicalMax));
        } else if v.UsagePage == PAGE_DIGITIZER && usage == USAGE_CONTACT_COUNT {
            count_collection = Some(v.LinkCollection);
        }
    }
    (!contacts.is_empty()).then_some(Device { preparsed, contacts, count_collection })
}

unsafe fn on_input(h: HRAWINPUT) {
    let header = std::mem::size_of::<RAWINPUTHEADER>() as u32;
    let mut size = 0u32;
    GetRawInputData(h, RID_INPUT, None, &mut size, header);
    if size == 0 || size > 64 * 1024 {
        return;
    }
    // u64 backing keeps RAWINPUT aligned.
    let mut buf = vec![0u64; (size as usize).div_ceil(8)];
    if GetRawInputData(h, RID_INPUT, Some(buf.as_mut_ptr() as _), &mut size, header) == u32::MAX {
        return;
    }
    let raw = &*(buf.as_ptr() as *const RAWINPUT);
    if raw.header.dwType != RIM_TYPEHID.0 {
        return;
    }
    let hid = &raw.data.hid;
    let data = std::slice::from_raw_parts(hid.bRawData.as_ptr(), (hid.dwSizeHid * hid.dwCount) as usize);
    let key = raw.header.hDevice.0 as isize;
    let step = DEVICES.with(|d| {
        let mut d = d.borrow_mut();
        let dev = d.entry(key).or_insert_with(|| load_device(raw.header.hDevice));
        let dev = dev.as_ref()?;
        let report_len = hid.dwSizeHid as usize;
        let mut step = None;
        for report in data.chunks(report_len.max(1)) {
            step = step.or(process(dev, report));
        }
        step
    });
    if let (Some(d), Some(app)) = (step, APP.get()) {
        app.shortcut(Shortcut::SpaceStep(d));
    }
}

/// Feeds one HID report into the gesture; returns a space step when a swipe completes.
unsafe fn process(dev: &Device, report: &[u8]) -> Option<i32> {
    let pp = PHIDP_PREPARSED_DATA(dev.preparsed.as_ptr() as _);
    let value = |page: u16, coll: u16, usage: u16| -> Option<u32> {
        let mut v = 0u32;
        HidP_GetUsageValue(HidP_Input, page, Some(coll), usage, &mut v, pp, report).is_ok().then_some(v)
    };
    let count = dev.count_collection.and_then(|c| value(PAGE_DIGITIZER, c, USAGE_CONTACT_COUNT)).unwrap_or(0);
    let mut samples = Vec::new();
    for &(coll, min, max) in &dev.contacts {
        let Some(x) = value(PAGE_GENERIC, coll, USAGE_X) else { continue };
        let id = value(PAGE_DIGITIZER, coll, USAGE_CONTACT_ID).unwrap_or(coll as u32);
        samples.push((id, (x as f64 - min as f64) / (max - min) as f64));
    }
    GESTURE.with(|g| {
        let mut g = g.borrow_mut();
        let now = Instant::now();
        if g.last.is_none_or(|t| now - t > IDLE) {
            *g = Gesture::default();
        }
        g.last = Some(now);
        // A non-zero count starts a new frame (hybrid reports send 0 after the first).
        if count > 0 {
            if count != g.fingers {
                g.xs.clear();
                g.start = None;
            }
            g.fingers = count;
        }
        for (id, x) in samples.into_iter().take(g.fingers as usize) {
            g.xs.insert(id, x);
        }
        if g.fingers != 4 || g.xs.len() < 4 {
            if g.fingers < 3 {
                g.fired = false;
            }
            return None;
        }
        let avg = g.xs.values().sum::<f64>() / g.xs.len() as f64;
        let start = *g.start.get_or_insert(avg);
        let delta = avg - start;
        if g.fired || delta.abs() < TRIGGER {
            return None;
        }
        g.fired = true;
        // Fingers moving left bring in the space to the right, like turning a page.
        Some(if delta < 0.0 { 1 } else { -1 })
    })
}
