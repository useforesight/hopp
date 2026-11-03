//! A real native text field for clipboard/input acceptance tests. It has no
//! network, media, portal or injected-input code. Run only in a private desktop.
use anyhow::{Context as _, Result, ensure};
use gpui_kit::base::input::InputState;
use gpui_kit::{
    AppContext, Bounds, Context, Entity, IntoElement, ParentElement, Render, Styled,
    TitlebarOptions, Window, WindowBounds, WindowOptions, div, px, size,
};
use gpui_omarchy::ActiveTheme;
use std::{path::PathBuf, time::Duration};

struct Editor {
    field: Entity<InputState>,
}
impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.omarchy().clone();
        div()
            .size_full()
            .p(px(24.))
            .bg(theme.background)
            .text_color(theme.foreground)
            .child("Hopp isolated native editor")
            .child(
                div()
                    .mt(px(24.))
                    .child(gpui_omarchy::input("editor", &self.field, window, cx)),
            )
    }
}
fn main() -> Result<()> {
    let runtime =
        PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").context("Private runtime required")?);
    ensure!(
        runtime.starts_with(std::env::temp_dir()),
        "Use a private test desktop runtime under /tmp"
    );
    // These devices/sockets are hidden by the compositor harness, preventing
    // this helper from accidentally becoming an actual-desktop test target.
    ensure!(
        !std::path::Path::new("/dev/input").exists(),
        "Physical input must be hidden"
    );
    ensure!(
        !std::path::Path::new("/run/user").exists(),
        "Actual desktop sockets must be hidden"
    );
    let state = runtime.join("hopp-editor.txt");
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_omarchy::init(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                            None,
                            size(px(900.), px(400.)),
                            cx,
                        ))),
                        titlebar: Some(TitlebarOptions {
                            title: Some("Hopp isolated native editor".into()),
                            ..Default::default()
                        }),
                        app_id: Some("hopp-native-editor".into()),
                        ..Default::default()
                    },
                    |window, cx| {
                        cx.new(|cx| {
                            let field = cx.new(|cx| {
                                InputState::new(window, cx).default_value("host clipboard seed")
                            });
                            field.update(cx, |field, cx| field.focus(window, cx));
                            cx.observe(&field, |_, _, cx| cx.notify()).detach();
                            Editor { field }
                        })
                    },
                )
                .expect("open isolated editor");
            cx.spawn(async move |cx| {
                for _ in 0..6000 {
                    cx.background_executor()
                        .timer(Duration::from_millis(50))
                        .await;
                    let value = window.update(cx, |editor, _, cx| {
                        editor.field.read(cx).value().to_string()
                    });
                    let Ok(value) = value else {
                        break;
                    };
                    if let Err(error) = std::fs::write(&state, value) {
                        eprintln!("Editor report: {error}");
                        break;
                    }
                }
                cx.update(|cx| cx.quit());
            })
            .detach();
            cx.activate(true);
        });
    Ok(())
}
