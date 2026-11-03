// Tray icon management with platform-specific implementations.
//
// macOS: Uses template images for automatic light/dark adaptation per display,
// with CALayer overlays for the colored dots: during calls, and for missed calls.
//
// Other platforms: No-op implementation (tray features not yet implemented).

use tauri::image::Image;
use tauri::path::BaseDirectory;
use tauri::tray::TrayIcon;
use tauri::{AppHandle, Manager, Wry};

// Platform-specific type alias
#[cfg(target_os = "macos")]
type PlatformTrayState = macos::MacOSTrayState;
#[cfg(not(target_os = "macos"))]
type PlatformTrayState = default::DefaultTrayState;

/// Platform-agnostic tray state manager.
pub struct TrayState {
    inner: PlatformTrayState,
}

impl TrayState {
    pub fn new(tray_icon: TrayIcon<Wry>) -> Self {
        Self {
            inner: PlatformTrayState::new(tray_icon),
        }
    }

    pub fn set_notification_enabled(&mut self, enabled: bool) {
        self.inner.set_notification_enabled(enabled);
    }

    pub fn is_notification_enabled(&self) -> bool {
        self.inner.is_notification_enabled()
    }

    pub fn set_missed_calls_enabled(&mut self, enabled: bool) {
        self.inner.set_missed_calls_enabled(enabled);
    }

    pub fn rect(&self) -> Option<tauri::Rect> {
        self.inner.rect()
    }
}

// =============================================================================
// Default (no-op) implementation for non-macOS platforms
// =============================================================================

#[cfg(not(target_os = "macos"))]
mod default {
    use super::*;

    pub struct DefaultTrayState {
        #[allow(dead_code)]
        tray_icon: TrayIcon<Wry>,
        notification_enabled: bool,
    }

    impl DefaultTrayState {
        pub fn new(tray_icon: TrayIcon<Wry>) -> Self {
            Self {
                tray_icon,
                notification_enabled: false,
            }
        }

        pub fn set_notification_enabled(&mut self, enabled: bool) {
            self.notification_enabled = enabled;
            // No-op: platform-specific tray features not implemented
        }

        pub fn is_notification_enabled(&self) -> bool {
            self.notification_enabled
        }

        pub fn set_missed_calls_enabled(&mut self, _enabled: bool) {
            // No-op: platform-specific tray features not implemented
        }

        pub fn rect(&self) -> Option<tauri::Rect> {
            self.tray_icon.rect().ok().flatten()
        }
    }
}

// =============================================================================
// macOS implementation using CALayer for notification dot overlay
// =============================================================================

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{msg_send, MainThreadMarker};
    use objc2_app_kit::NSStatusBar;
    use objc2_foundation::{ns_string, NSPoint, NSRect, NSSize, NSString};
    use objc2_quartz_core::CALayer;

    pub struct MacOSTrayState {
        #[allow(dead_code)]
        tray_icon: TrayIcon<Wry>,
        notification_enabled: bool,
    }

    impl MacOSTrayState {
        pub fn new(tray_icon: TrayIcon<Wry>) -> Self {
            Self {
                tray_icon,
                notification_enabled: false,
            }
        }

        pub fn set_notification_enabled(&mut self, enabled: bool) {
            self.notification_enabled = enabled;
            update_dot(
                &TrayDot {
                    layer_name: ns_string!("notificationDot"),
                    size: 4.0,
                    position: |bounds| NSPoint::new(10.5, bounds.size.height - 4.0 - 13.5),
                    // Green color: #05df72
                    color: (0.02, 0.875, 0.447),
                },
                enabled,
            );
        }

        pub fn is_notification_enabled(&self) -> bool {
            self.notification_enabled
        }

        pub fn set_missed_calls_enabled(&mut self, enabled: bool) {
            update_dot(
                &TrayDot {
                    layer_name: ns_string!("missedCallDot"),
                    size: 7.0,
                    // The top-right corner of the 18pt icon, which is centered in the button
                    position: |bounds| {
                        NSPoint::new(
                            bounds.size.width / 2.0 + 2.5,
                            bounds.size.height / 2.0 - 9.5,
                        )
                    },
                    // Red color: #f03333
                    color: (0.94, 0.2, 0.2),
                },
                enabled,
            );
        }

        pub fn rect(&self) -> Option<tauri::Rect> {
            self.tray_icon.rect().ok().flatten()
        }
    }

    struct TrayDot {
        layer_name: &'static NSString,
        size: f64,
        /// Coordinates from top-left of button
        position: fn(NSRect) -> NSPoint,
        color: (f64, f64, f64),
    }

    /// Add or remove a colored dot overlay on the tray icon button using CALayer.
    /// This preserves the template behavior of the base icon while adding color.
    fn update_dot(dot_style: &TrayDot, show: bool) {
        unsafe {
            let Some(_mtm) = MainThreadMarker::new() else {
                log::warn!("[TRAY] update_dot: not on main thread");
                return;
            };

            let status_bar = NSStatusBar::systemStatusBar();

            // Access status items via private API (NSPointerArray)
            let items: *const AnyObject =
                msg_send![&*status_bar, valueForKey: objc2_foundation::ns_string!("_statusItems")];
            if items.is_null() {
                return;
            }

            let count: usize = msg_send![items, count];

            // We might get >1 item for our app, but its filtered from the image selector.
            for i in 0..count {
                let item: *const AnyObject = msg_send![items, pointerAtIndex: i];
                if item.is_null() {
                    continue;
                }

                let button: *const AnyObject = msg_send![item, button];
                if button.is_null() {
                    continue;
                }

                // Only process items with a template image (our tray icon)
                let image: *const AnyObject = msg_send![button, image];
                if image.is_null() {
                    continue;
                }

                // Log if it's a template image
                let is_template: bool = msg_send![image, isTemplate];
                if !is_template {
                    continue;
                }

                let _: () = msg_send![button, setWantsLayer: true];
                let layer: *const AnyObject = msg_send![button, layer];
                if layer.is_null() {
                    continue;
                }

                let bounds: NSRect = msg_send![button, bounds];

                // Look for existing dot layer by name
                let dot_layer_name = dot_style.layer_name;
                let sublayers: *const AnyObject = msg_send![layer, sublayers];

                let mut existing_dot: *const AnyObject = std::ptr::null();
                if !sublayers.is_null() {
                    let sublayer_count: usize = msg_send![sublayers, count];
                    for j in 0..sublayer_count {
                        let sublayer: *const AnyObject = msg_send![sublayers, objectAtIndex: j];
                        let name: *const AnyObject = msg_send![sublayer, name];
                        if !name.is_null() {
                            let is_equal: bool = msg_send![name, isEqualToString: dot_layer_name];
                            if is_equal {
                                existing_dot = sublayer;
                                break;
                            }
                        }
                    }
                }

                if show {
                    if existing_dot.is_null() {
                        let dot = CALayer::new();
                        let _: () = msg_send![&*dot, setName: dot_layer_name];

                        let dot_size = dot_style.size;
                        let dot_frame = NSRect::new(
                            (dot_style.position)(bounds),
                            NSSize::new(dot_size, dot_size),
                        );
                        dot.setFrame(dot_frame);

                        let (red, green, blue) = dot_style.color;
                        let ns_color_class = objc2::runtime::AnyClass::get(c"NSColor").unwrap();
                        let color: Retained<AnyObject> = msg_send![
                            ns_color_class,
                            colorWithSRGBRed: red,
                            green: green,
                            blue: blue,
                            alpha: 1.0_f64
                        ];
                        let cg_color: *const AnyObject = msg_send![&*color, CGColor];

                        let _: () = msg_send![&*dot, setBackgroundColor: cg_color];
                        dot.setCornerRadius(dot_size / 2.0);

                        let _: () = msg_send![layer, addSublayer: &*dot];
                    } else {
                        let _: () = msg_send![existing_dot, setHidden: false];
                    }
                } else if !existing_dot.is_null() {
                    let _: () = msg_send![existing_dot, setHidden: true];
                }
            }
        }
    }
}

// =============================================================================
// Shared utilities
// =============================================================================

/// Load a tray icon from bundled resources.
/// Only used during initial setup in `setup_tray_icon()`.
pub fn load_tray_icon(app_handle: &AppHandle, filename: &str) -> Option<Image<'static>> {
    let icon_path = app_handle
        .path()
        .resolve(
            format!("resources/tray-icons/{}", filename),
            BaseDirectory::Resource,
        )
        .ok()?;

    let icon_bytes = std::fs::read(&icon_path).ok()?;
    Image::from_bytes(&icon_bytes).ok()
}
