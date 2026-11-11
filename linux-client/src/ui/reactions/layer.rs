//! The reactions' layer: one transparent layer-shell surface over the whole monitor with
//! the call window, above everything, that takes no input and is kept out of screen
//! shares by Hopp's Hyprland rules. It's opened for a reaction and closed once nothing
//! has played for a while (see [`super::Idle`]).
use std::{cell::RefCell, rc::Rc};

use gpui_kit::{
    AppContext, Bounds, Context, Entity, IntoElement, ParentElement, Pixels, Render, Styled,
    Window, WindowHandle, canvas, div,
    layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions},
    px, size,
};

use super::{Reactions, scene::Pictures};

pub const NAMESPACE: &str = "hopp-reactions";

pub struct ReactionsLayer {
    reactions: Entity<Reactions>,
    pictures: Rc<RefCell<Pictures>>,
}

impl ReactionsLayer {
    fn new(reactions: Entity<Reactions>, cx: &mut Context<Self>) -> Self {
        cx.observe(&reactions, |_, _, cx| cx.notify()).detach();
        Self {
            reactions,
            pictures: Default::default(),
        }
    }
}

impl Render for ReactionsLayer {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let reactions = self.reactions.clone();
        let pictures = self.pictures.clone();
        div().size_full().child(
            canvas(
                |bounds, _, _| bounds,
                move |_, bounds: Bounds<Pixels>, window, cx| {
                    let (frame, busy) = reactions.update(cx, |reactions, _| {
                        reactions
                            .resize([f32::from(bounds.size.width), f32::from(bounds.size.height)]);
                        (reactions.frame(), reactions.player.busy())
                    });
                    let origin = [f32::from(bounds.origin.x), f32::from(bounds.origin.y)];
                    super::scene::paint(window, &mut pictures.borrow_mut(), &frame, origin);
                    if busy {
                        window.request_animation_frame();
                    }
                },
            )
            .size_full(),
        )
    }
}

/// Open the layer over monitor `output` (`width`×`height` logical pixels), or wherever
/// the compositor puts it when that isn't known.
pub fn open(
    reactions: Entity<Reactions>,
    output: Option<&str>,
    (width, height): (f32, f32),
    cx: &mut gpui_kit::App,
) -> Option<WindowHandle<ReactionsLayer>> {
    let display_id = output.and_then(|name| super::super::overlay::display_named(name, cx));
    let options = gpui_kit::WindowOptions {
        window_bounds: Some(gpui_kit::WindowBounds::Windowed(Bounds {
            origin: Default::default(),
            size: size(px(width), px(height)),
        })),
        kind: gpui_kit::WindowKind::LayerShell(LayerShellOptions {
            namespace: NAMESPACE.into(),
            layer: Layer::Overlay,
            anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            // Over the bar too: reactions fill the screen.
            exclusive_zone: Some(px(-1.)),
            exclusive_edge: None,
            margin: None,
            keyboard_interactivity: KeyboardInteractivity::None,
        }),
        display_id,
        app_id: Some("hopp.reactions".into()),
        focus: false,
        window_background: gpui_kit::WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };
    let layer_reactions = reactions.clone();
    let handle = match cx.open_window(options, |_, cx| {
        cx.new(|cx| ReactionsLayer::new(layer_reactions, cx))
    }) {
        Ok(handle) => handle,
        // Compositors without layer-shell (the screenshot harness) get a plain window.
        Err(_) => cx
            .open_window(
                gpui_kit::WindowOptions {
                    window_bounds: Some(gpui_kit::WindowBounds::Windowed(Bounds {
                        origin: Default::default(),
                        size: size(px(width), px(height)),
                    })),
                    app_id: Some("hopp.reactions".into()),
                    focus: false,
                    window_background: gpui_kit::WindowBackgroundAppearance::Transparent,
                    ..Default::default()
                },
                |_, cx| cx.new(|cx| ReactionsLayer::new(reactions, cx)),
            )
            .ok()?,
    };
    // Clicks and scrolling go through to whatever is underneath.
    let _ = handle.update(cx, |_, window, _| window.set_input_region(Some(&[])));
    Some(handle)
}
