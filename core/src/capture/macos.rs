use std::collections::HashMap;
use std::sync::Arc;

use core_foundation::{
    base::{CFType, TCFType},
    dictionary::CFDictionary,
    string::CFString,
};
use core_graphics::{
    display::CGDisplay,
    event::CGEvent,
    event_source::{CGEventSource, CGEventSourceStateID},
    geometry::CGRect,
    window::{
        copy_window_info, create_window_list, kCGNullWindowID, kCGWindowBounds,
        kCGWindowListExcludeDesktopElements, kCGWindowListOptionOnScreenOnly, kCGWindowOwnerPID,
        kCGWindowSharingState,
    },
};
use hopp_capture::moments::OwnWindows;
use screencapturekit::shareable_content::SCShareableContent;
#[cfg(target_os = "macos")]
use winit::platform::macos::MonitorHandleExtMacOS;

use crate::{
    app_veil::number,
    capture::capturer::{MonitorId, ScreenshareExt},
    utils::geometry::{Extent, Frame, Position},
    SelectableWindow,
};

pub struct ScreenshareFunctions {}

impl ScreenshareExt for ScreenshareFunctions {
    fn get_selected_monitor(
        monitors: &[winit::monitor::MonitorHandle],
        input_id: u32,
    ) -> winit::monitor::MonitorHandle {
        let mut selected_monitor = monitors[0].clone();
        for monitor in monitors {
            if monitor.native_id() == input_id {
                selected_monitor = monitor.clone();
            }
        }
        selected_monitor
    }

    fn get_monitor_id(monitor: &winit::monitor::MonitorHandle) -> MonitorId {
        MonitorId::Numeric(monitor.native_id())
    }

    fn capture_content_id_for_monitor(monitor: &winit::monitor::MonitorHandle) -> Option<u32> {
        Some(monitor.native_id())
    }
}

impl Default for ScreenshareFunctions {
    fn default() -> Self {
        Self::new()
    }
}

impl ScreenshareFunctions {
    pub fn new() -> Self {
        Self {}
    }

    pub(crate) fn cursor_position() -> Option<Position> {
        let event_source = match CGEventSource::new(CGEventSourceStateID::CombinedSessionState) {
            Ok(event_source) => event_source,
            Err(()) => {
                log::error!("cursor_position: failed to create CoreGraphics event source");
                return None;
            }
        };
        let cursor = match CGEvent::new(event_source) {
            Ok(event) => event.location(),
            Err(()) => {
                log::error!("cursor_position: failed to read cursor position");
                return None;
            }
        };

        Some(Position {
            x: cursor.x,
            y: cursor.y,
        })
    }

    pub(crate) fn selectable_windows() -> Option<Vec<SelectableWindow>> {
        let content = match SCShareableContent::create()
            .with_exclude_desktop_windows(true)
            .with_on_screen_windows_only(true)
            .get()
        {
            Ok(content) => content,
            Err(error) => {
                log::error!("selectable_windows: ScreenCaptureKit enumeration failed: {error:?}");
                return None;
            }
        };
        let current_process_id = std::process::id() as i32;
        let windows: HashMap<u32, Frame> = content
            .windows()
            .into_iter()
            .filter(|window| {
                let owner_process_id = window
                    .owning_application()
                    .map(|application| application.process_id());
                is_window_selectable(window.window_layer(), owner_process_id, current_process_id)
            })
            .map(|window| {
                let frame = window.frame();
                (
                    window.window_id(),
                    Frame {
                        origin_x: frame.origin.x,
                        origin_y: frame.origin.y,
                        extent: Extent {
                            width: frame.size.width,
                            height: frame.size.height,
                        },
                    },
                )
            })
            .collect();

        let Some(window_ids) = create_window_list(
            kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
            kCGNullWindowID,
        ) else {
            log::error!("selectable_windows: CoreGraphics returned no window list");
            return None;
        };

        Some(windows_in_front_order(
            &windows,
            window_ids.iter().map(|window_id| *window_id),
        ))
    }
}

impl ScreenshareFunctions {
    /// Finds Hopp's windows that a share of `display_id` shows, as fractions of it. Most of
    /// them are hidden from shares, so can't change what's shared and would only hide real
    /// changes beneath them: in practice this is the camera window.
    pub fn own_windows_on_display(display_id: u32) -> OwnWindows {
        let own_processes = [
            std::process::id() as i32,
            std::os::unix::process::parent_id() as i32,
        ];
        Arc::new(move || -> Option<Vec<[f64; 4]>> {
            let display = CGDisplay::new(display_id).bounds();
            let descriptions = copy_window_info(
                kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
                kCGNullWindowID,
            )?;
            let bounds_key = unsafe { CFString::wrap_under_get_rule(kCGWindowBounds) };
            let windows: Vec<[f64; 4]> = descriptions
                .get_all_values()
                .into_iter()
                .filter_map(|description| {
                    let description = unsafe {
                        CFDictionary::<CFString, CFType>::wrap_under_get_rule(description as _)
                    };
                    let owner = number(&description, unsafe { kCGWindowOwnerPID })?;
                    let shown = number(&description, unsafe { kCGWindowSharingState })? != 0;
                    if !own_processes.contains(&owner) || !shown {
                        return None;
                    }
                    let bounds = description.find(&bounds_key)?;
                    let bounds =
                        unsafe { CFDictionary::wrap_under_get_rule(bounds.as_CFTypeRef() as _) };
                    fraction_of_display(CGRect::from_dict_representation(&bounds)?, display)
                })
                .collect();
            Some(windows)
        })
    }
}

/// `[left, top, right, bottom]` of a window as fractions of a display, if it's on it.
fn fraction_of_display(window: CGRect, display: CGRect) -> Option<[f64; 4]> {
    if display.size.width <= 0.0 || display.size.height <= 0.0 {
        return None;
    }
    let left = (window.origin.x - display.origin.x) / display.size.width;
    let top = (window.origin.y - display.origin.y) / display.size.height;
    let right = left + window.size.width / display.size.width;
    let bottom = top + window.size.height / display.size.height;
    (right > left && bottom > top && right > 0.0 && bottom > 0.0 && left < 1.0 && top < 1.0)
        .then_some([left, top, right, bottom])
}

fn is_window_selectable(
    layer: i32,
    owner_process_id: Option<i32>,
    current_process_id: i32,
) -> bool {
    layer == 0 && owner_process_id.is_some_and(|process_id| process_id != current_process_id)
}

fn windows_in_front_order(
    windows: &HashMap<u32, Frame>,
    window_ids: impl IntoIterator<Item = u32>,
) -> Vec<SelectableWindow> {
    window_ids
        .into_iter()
        .filter_map(|id| {
            windows
                .get(&id)
                .copied()
                .map(|frame| SelectableWindow { id, frame })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_graphics::geometry::{CGPoint, CGSize};

    #[test]
    fn a_window_is_a_fraction_of_the_display_it_is_on() {
        let rect =
            |x, y, width, height| CGRect::new(&CGPoint::new(x, y), &CGSize::new(width, height));
        // The display right of the main one.
        let display = rect(1000., 0., 2000., 1000.);
        assert_eq!(
            fraction_of_display(rect(2500., 0., 500., 1000.), display),
            Some([0.75, 0., 1., 1.])
        );
        // Hanging over its edge, it covers what it covers.
        assert_eq!(
            fraction_of_display(rect(800., 0., 400., 500.), display),
            Some([-0.1, 0., 0.1, 0.5])
        );
        // On the main display.
        assert_eq!(fraction_of_display(rect(0., 0., 800., 500.), display), None);
        // A display that has gone away.
        assert_eq!(
            fraction_of_display(rect(0., 0., 800., 500.), rect(0., 0., 0., 0.)),
            None
        );
    }

    #[test]
    fn joins_shareable_windows_in_front_to_back_order() {
        let frame = Frame {
            origin_x: 0.0,
            origin_y: 0.0,
            extent: Extent {
                width: 100.0,
                height: 100.0,
            },
        };
        let windows = HashMap::from([(1, frame), (2, frame), (3, frame)]);

        let ordered = windows_in_front_order(&windows, [3, 9, 1]);

        assert_eq!(
            ordered
                .into_iter()
                .map(|window| window.id)
                .collect::<Vec<_>>(),
            vec![3, 1]
        );
    }
}
