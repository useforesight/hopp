//! While you share, a small pill at the top of the shared display says so and holds what you
//! reach for: the pen, stamps, taking back control, stopping.
//!
//! It's a panel that never activates, so using it doesn't take the focus from what you're
//! showing, and it's kept out of the share like the overlay is.

use std::cell::Cell;

use objc2::{
    rc::Retained, runtime::AnyObject, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameDarkAqua,
    NSAttributedStringNSStringDrawing, NSBackingStoreType, NSBox, NSBoxType, NSButton,
    NSButtonType, NSColor, NSEvent, NSFont, NSFontAttributeName, NSForegroundColorAttributeName,
    NSLineBreakMode, NSPanel, NSPopUpMenuWindowLevel, NSRunningApplication, NSScreen, NSTextField,
    NSTitlePosition, NSTrackingArea, NSTrackingAreaOptions, NSView, NSWindowCollectionBehavior,
    NSWindowSharingType, NSWindowStyleMask,
};
use objc2_foundation::{
    NSAttributedString, NSDictionary, NSNumber, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};

use crate::app_veil::panel::{color, register_font};
use crate::windows::colors::ColorToken;

const HEIGHT: f64 = 40.0;
const MAX_WIDTH: f64 = 520.0;
/// Below the menu bar.
const TOP_GAP: f64 = 8.0;
const BUTTON_HEIGHT: f64 = 28.0;
const BUTTON_PADDING: f64 = 12.0;
const BUTTON_GAP: f64 = 4.0;
/// Around the buttons, so that their corners follow the pill's.
const INSET: f64 = (HEIGHT - BUTTON_HEIGHT) / 2.0;
const DOT_SIZE: f64 = 8.0;
const DOT_RING_SIZE: f64 = 14.0;
const DOT_X: f64 = 16.0;
const LABEL_X: f64 = DOT_X + DOT_SIZE + 8.0;
const LABEL_GAP: f64 = 12.0;
const FONT_SIZE: f64 = 13.0;

const ALLOW: &str = "Allow";
const REVOKE: &str = "Revoke";

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharingPillClick {
    Draw,
    Stamp,
    Control,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SharingPillState {
    pub drawing: bool,
    pub stamping: bool,
    pub control_allowed: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Look {
    Plain,
    On,
    Danger,
}

struct ButtonIvars {
    clicked: Box<dyn Fn()>,
    background: Retained<NSBox>,
    look: Cell<Look>,
    hovered: Cell<bool>,
}

objc2::define_class!(
    #[unsafe(super = NSButton)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ButtonIvars]
    struct PillButton;

    unsafe impl NSObjectProtocol for PillButton {}

    impl PillButton {
        #[unsafe(method(pillClicked:))]
        fn pill_clicked(&self, _sender: &NSButton) {
            (self.ivars().clicked)();
        }

        // This window is never the active one, and the first click is the only one it gets.
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: &NSEvent) {
            self.ivars().hovered.set(true);
            self.paint();
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            self.ivars().hovered.set(false);
            self.paint();
        }
    }
);

impl PillButton {
    fn new(
        mtm: MainThreadMarker,
        frame: NSRect,
        title: &str,
        look: Look,
        clicked: Box<dyn Fn()>,
    ) -> (Retained<NSBox>, Retained<Self>) {
        let background = NSBox::initWithFrame(NSBox::alloc(mtm), frame);
        background.setBoxType(NSBoxType::Custom);
        background.setTitlePosition(NSTitlePosition::NoTitle);
        background.setContentViewMargins(NSSize::new(0.0, 0.0));
        background.setBorderWidth(0.0);
        background.setCornerRadius(BUTTON_HEIGHT / 2.0);

        let this = Self::alloc(mtm).set_ivars(ButtonIvars {
            clicked,
            background: background.clone(),
            look: Cell::new(look),
            hovered: Cell::new(false),
        });
        let this: Retained<Self> = unsafe {
            objc2::msg_send![super(this), initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), frame.size)]
        };
        this.setButtonType(NSButtonType::MomentaryChange);
        this.setBordered(false);
        unsafe {
            // The button owns itself through the content view; the target is weak.
            this.setTarget(Some(&this));
            this.setAction(Some(objc2::sel!(pillClicked:)));
        }
        let hover = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                this.bounds(),
                NSTrackingAreaOptions::MouseEnteredAndExited
                    | NSTrackingAreaOptions::ActiveAlways
                    | NSTrackingAreaOptions::InVisibleRect,
                Some(&this),
                None,
            )
        };
        this.addTrackingArea(&hover);
        this.show_title(title);
        this.paint();
        if let Some(content) = background.contentView() {
            content.addSubview(&this);
        }
        (background, this)
    }

    fn show_title(&self, title: &str) {
        self.setAttributedTitle(&styled(title));
    }

    fn set_look(&self, look: Look) {
        if self.ivars().look.replace(look) != look {
            self.paint();
        }
    }

    fn paint(&self) {
        let ivars = self.ivars();
        let fill = match (ivars.look.get(), ivars.hovered.get()) {
            (Look::Plain, false) => NSColor::colorWithWhite_alpha(1.0, 0.1),
            (Look::Plain, true) => NSColor::colorWithWhite_alpha(1.0, 0.18),
            (Look::On, false) => color(ColorToken::Orange500),
            (Look::On, true) => color(ColorToken::Orange400),
            (Look::Danger, false) => color(ColorToken::Red500),
            (Look::Danger, true) => color(ColorToken::Red400),
        };
        ivars.background.setFillColor(&fill);
    }
}

fn font() -> Retained<NSFont> {
    NSFont::fontWithName_size(&NSString::from_str("Geist-Medium"), FONT_SIZE)
        .unwrap_or_else(|| NSFont::systemFontOfSize(FONT_SIZE))
}

fn styled(text: &str) -> Retained<NSAttributedString> {
    let font = font();
    let white = NSColor::whiteColor();
    let attributes = unsafe {
        NSDictionary::from_slices(
            &[NSFontAttributeName, NSForegroundColorAttributeName],
            &[&*font as &AnyObject, &*white as &AnyObject],
        )
    };
    unsafe {
        NSAttributedString::initWithString_attributes(
            NSAttributedString::alloc(),
            &NSString::from_str(text),
            Some(&attributes),
        )
    }
}

fn button_width(titles: &[&str]) -> f64 {
    titles
        .iter()
        .map(|title| styled(title).size().width.ceil())
        .fold(0.0, f64::max)
        + BUTTON_PADDING * 2.0
}

/// How wide the pill is and how much of the label it can show, for buttons of these widths.
fn fit(label_width: f64, button_widths: &[f64]) -> (f64, f64) {
    let buttons = button_widths.iter().sum::<f64>()
        + BUTTON_GAP * button_widths.len().saturating_sub(1) as f64;
    let others = LABEL_X + LABEL_GAP + buttons + INSET;
    let shown = label_width.min(MAX_WIDTH - others).max(0.0);
    (others + shown, shown)
}

fn screen_for(mtm: MainThreadMarker, display_id: u32) -> Option<Retained<NSScreen>> {
    let key = NSString::from_str("NSScreenNumber");
    NSScreen::screens(mtm)
        .iter()
        .find(|screen| {
            screen
                .deviceDescription()
                .objectForKey(&key)
                .and_then(|number| number.downcast::<NSNumber>().ok())
                .is_some_and(|number| number.unsignedIntValue() == display_id)
        })
        .or_else(|| NSScreen::mainScreen(mtm))
}

pub struct SharingPill {
    panel: Retained<NSPanel>,
    draw: Retained<PillButton>,
    stamp: Retained<PillButton>,
    control: Option<Retained<PillButton>>,
    state: Cell<Option<SharingPillState>>,
}

impl Drop for SharingPill {
    fn drop(&mut self) {
        self.panel.close();
    }
}

impl SharingPill {
    /// `window_owner` is whose window is being shared, if it isn't a whole display.
    pub fn new(
        display_id: u32,
        window_owner: Option<i32>,
        clicked: impl Fn(SharingPillClick) + Clone + 'static,
    ) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        let screen = screen_for(mtm, display_id)?;
        register_font();

        let shared = match window_owner {
            Some(process_id) => {
                NSRunningApplication::runningApplicationWithProcessIdentifier(process_id)
                    .and_then(|application| application.localizedName())
                    .map_or("a window".to_string(), |name| name.to_string())
            }
            None => screen.localizedName().to_string(),
        };
        let title = NSString::from_str(&format!("Sharing {shared}"));
        let label = NSTextField::labelWithString(&title, mtm);
        label.setFont(Some(&font()));
        label.setTextColor(Some(&NSColor::whiteColor()));
        label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        label.setToolTip(Some(&title));
        let label_size = label.intrinsicContentSize();

        // Without this permission nobody could control your screen anyway.
        let can_control = unsafe { AXIsProcessTrusted() };
        let draw_width = button_width(&["Draw"]);
        let stamp_width = button_width(&["Stamp"]);
        let control_width = button_width(&[ALLOW, REVOKE]);
        let stop_width = button_width(&["Stop"]);
        let mut widths = vec![draw_width, stamp_width];
        if can_control {
            widths.push(control_width);
        }
        widths.push(stop_width);
        let (width, label_width) = fit(label_size.width.ceil(), &widths);

        let visible = screen.visibleFrame();
        let whole = screen.frame();
        let frame = NSRect::new(
            NSPoint::new(
                (whole.origin.x + (whole.size.width - width) / 2.0).round(),
                (visible.origin.y + visible.size.height - TOP_GAP - HEIGHT).round(),
            ),
            NSSize::new(width, HEIGHT),
        );
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            frame,
            NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            NSBackingStoreType::Buffered,
            false,
        );
        unsafe { panel.setReleasedWhenClosed(false) };
        panel.setFloatingPanel(true);
        panel.setHidesOnDeactivate(false);
        panel.setBecomesKeyOnlyIfNeeded(true);
        // Above the draw pad and the overlay, which are above the menu bar.
        panel.setLevel(NSPopUpMenuWindowLevel);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(true);
        panel.setSharingType(NSWindowSharingType::None);

        let background = NSBox::initWithFrame(
            NSBox::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), frame.size),
        );
        background.setBoxType(NSBoxType::Custom);
        background.setTitlePosition(NSTitlePosition::NoTitle);
        background.setContentViewMargins(NSSize::new(0.0, 0.0));
        background.setBorderWidth(1.0);
        background.setCornerRadius(HEIGHT / 2.0);
        background.setBorderColor(&NSColor::colorWithWhite_alpha(1.0, 0.12));
        background.setFillColor(&color(ColorToken::Zinc900));
        background.setAppearance(
            NSAppearance::appearanceNamed(unsafe { NSAppearanceNameDarkAqua }).as_deref(),
        );
        panel.setContentView(Some(&background));
        let content: Retained<NSView> = background.contentView()?;

        let dot = |size: f64, alpha: f64| {
            let dot = NSBox::initWithFrame(
                NSBox::alloc(mtm),
                NSRect::new(
                    NSPoint::new(DOT_X + (DOT_SIZE - size) / 2.0, (HEIGHT - size) / 2.0),
                    NSSize::new(size, size),
                ),
            );
            dot.setBoxType(NSBoxType::Custom);
            dot.setTitlePosition(NSTitlePosition::NoTitle);
            dot.setBorderWidth(0.0);
            dot.setCornerRadius(size / 2.0);
            let red = ColorToken::Red500.to_color();
            dot.setFillColor(&NSColor::colorWithSRGBRed_green_blue_alpha(
                red.r as f64,
                red.g as f64,
                red.b as f64,
                alpha,
            ));
            dot
        };
        content.addSubview(&dot(DOT_RING_SIZE, 0.25));
        content.addSubview(&dot(DOT_SIZE, 1.0));

        label.setFrame(NSRect::new(
            NSPoint::new(LABEL_X, ((HEIGHT - label_size.height) / 2.0).round()),
            NSSize::new(label_width, label_size.height),
        ));
        content.addSubview(&label);

        let mut x = LABEL_X + label_width + LABEL_GAP;
        let mut add = |title: &str, width: f64, look: Look, click: SharingPillClick| {
            let clicked = clicked.clone();
            let (holder, button) = PillButton::new(
                mtm,
                NSRect::new(NSPoint::new(x, INSET), NSSize::new(width, BUTTON_HEIGHT)),
                title,
                look,
                Box::new(move || clicked(click)),
            );
            content.addSubview(&holder);
            x += width + BUTTON_GAP;
            button
        };
        let draw = add("Draw", draw_width, Look::Plain, SharingPillClick::Draw);
        let stamp = add("Stamp", stamp_width, Look::Plain, SharingPillClick::Stamp);
        let control = can_control.then(|| {
            add(
                REVOKE,
                control_width,
                Look::Plain,
                SharingPillClick::Control,
            )
        });
        add("Stop", stop_width, Look::Danger, SharingPillClick::Stop);

        panel.orderFrontRegardless();
        panel.invalidateShadow();
        Some(Self {
            panel,
            draw,
            stamp,
            control,
            state: Cell::new(None),
        })
    }

    pub fn update(&self, state: SharingPillState) {
        if self.state.replace(Some(state)) == Some(state) {
            return;
        }
        log::info!("sharing_pill: {state:?}");
        let tool = |on: bool| if on { Look::On } else { Look::Plain };
        self.draw.set_look(tool(state.drawing));
        self.stamp.set_look(tool(state.stamping));
        if let Some(control) = &self.control {
            control.show_title(if state.control_allowed { REVOKE } else { ALLOW });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_name_keeps_the_pill_small() {
        let (width, shown) = fit(100.0, &[60.0, 60.0, 60.0, 60.0]);
        assert_eq!(shown, 100.0);
        assert!(width < MAX_WIDTH, "{width}");
    }

    #[test]
    fn a_long_name_gives_way_to_the_buttons() {
        let (width, shown) = fit(900.0, &[60.0, 60.0, 60.0, 60.0]);
        assert_eq!(width, MAX_WIDTH);
        assert!(shown > 100.0, "{shown}");
        assert!(shown < 900.0, "{shown}");
    }
}
