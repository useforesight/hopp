//! Noticing that someone has stepped away from their computer, so teammates can see they're Away.

use std::time::Duration;

/// How soon a locked screen, or someone coming back, shows up.
pub const CHECK_EVERY: Duration = Duration::from_secs(5);

/// Whether the screen is locked, or nothing has been touched for this many minutes.
pub fn has_stepped_away(idle_minutes: u32) -> bool {
    is_screen_locked() || seconds_since_input() >= f64::from(idle_minutes) * 60.
}

#[cfg(target_os = "macos")]
mod platform {
    use objc2::rc::Retained;
    use objc2_foundation::{ns_string, NSDictionary, NSNumber, NSObject, NSString};

    /// `kCGEventSourceStateCombinedSessionState`
    const COMBINED_SESSION_STATE: i32 = 0;
    /// `kCGAnyInputEventType`
    const ANY_INPUT_EVENT: u32 = !0;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
        fn CGSessionCopyCurrentDictionary() -> *mut NSDictionary<NSString, NSObject>;
    }

    pub(super) fn seconds_since_input() -> f64 {
        unsafe { CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION_STATE, ANY_INPUT_EVENT) }
    }

    pub(super) fn is_screen_locked() -> bool {
        // SAFETY: the function follows the Create rule and a CFDictionary is an NSDictionary.
        let Some(session) = (unsafe { Retained::from_raw(CGSessionCopyCurrentDictionary()) })
        else {
            return false;
        };
        session
            .objectForKey(ns_string!("CGSSessionScreenIsLocked"))
            .and_then(|locked| locked.downcast::<NSNumber>().ok())
            .is_some_and(|locked| locked.as_bool())
    }
}

#[cfg(windows)]
mod platform {
    #[repr(C)]
    struct LastInputInfo {
        size: u32,
        ticks: u32,
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetLastInputInfo(info: *mut LastInputInfo) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetTickCount() -> u32;
    }

    pub(super) fn seconds_since_input() -> f64 {
        let mut info = LastInputInfo {
            size: std::mem::size_of::<LastInputInfo>() as u32,
            ticks: 0,
        };
        if unsafe { GetLastInputInfo(&mut info) } == 0 {
            return 0.;
        }
        let ticks = unsafe { GetTickCount() };
        f64::from(ticks.wrapping_sub(info.ticks)) / 1000.
    }

    /// Windows has no simple check for this. A locked screen takes no input, so it counts as idle.
    pub(super) fn is_screen_locked() -> bool {
        false
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    pub(super) fn seconds_since_input() -> f64 {
        0.
    }

    pub(super) fn is_screen_locked() -> bool {
        false
    }
}

use platform::{is_screen_locked, seconds_since_input};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_answers_without_crashing() {
        assert!(seconds_since_input() >= 0.);
        let _ = is_screen_locked();
    }
}
