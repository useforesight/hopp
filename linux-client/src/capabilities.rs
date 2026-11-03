//! Probe advertised interfaces only; never capture or inject input here.
use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::Serialize;
use wayland_client::{Connection, Dispatch, QueueHandle, protocol::wl_registry};

const PORTAL_BUS: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";

#[derive(Debug, Default, Serialize)]
pub struct Capabilities {
    pub desktop: String,
    pub globals: BTreeMap<String, u32>,
    pub screen_cast_version: Option<u32>,
    pub remote_desktop_version: Option<u32>,
    pub errors: Vec<String>,
}

impl Capabilities {
    pub fn probe() -> Self {
        let mut report = Self {
            desktop: std::env::var("XDG_CURRENT_DESKTOP")
                .unwrap_or_else(|_| "Unknown desktop".into()),
            ..Self::default()
        };
        match globals() {
            Ok(globals) => report.globals = globals,
            Err(error) => report.errors.push(format!("Wayland: {error:#}")),
        }
        if let Err(error) = report.probe_portals() {
            report.errors.push(format!("Desktop portal: {error:#}"));
        }
        report
    }

    fn probe_portals(&mut self) -> Result<()> {
        let connection = zbus::blocking::Connection::session()?;
        let proxy = zbus::blocking::Proxy::new(
            &connection,
            PORTAL_BUS,
            PORTAL_PATH,
            "org.freedesktop.DBus.Introspectable",
        )?;
        let xml: String = proxy.call("Introspect", &())?;
        // An absent interface is different from a broken property request.
        for (name, version) in [
            ("ScreenCast", &mut self.screen_cast_version),
            ("RemoteDesktop", &mut self.remote_desktop_version),
        ] {
            let interface = format!("org.freedesktop.portal.{name}");
            if xml.contains(&format!("interface name=\"{interface}\"")) {
                let proxy = zbus::blocking::Proxy::new(
                    &connection,
                    PORTAL_BUS,
                    PORTAL_PATH,
                    interface.as_str(),
                )?;
                *version = Some(proxy.get_property("version")?);
            }
        }
        Ok(())
    }

    pub fn virtual_pointer(&self) -> bool {
        self.globals.contains_key("zwlr_virtual_pointer_manager_v1")
    }

    pub fn virtual_keyboard(&self) -> bool {
        self.globals.contains_key("zwp_virtual_keyboard_manager_v1")
    }

    pub fn layer_shell(&self) -> bool {
        self.globals.contains_key("zwlr_layer_shell_v1")
    }

    pub fn input_route(&self) -> &'static str {
        if self.remote_desktop_version.is_some() {
            "RemoteDesktop portal advertised"
        } else if self.virtual_pointer() && self.virtual_keyboard() {
            "Wayland virtual pointer + keyboard advertised"
        } else {
            "No complete input route advertised"
        }
    }
}

#[derive(Default)]
struct RegistryState(BTreeMap<String, u32>);

impl Dispatch<wl_registry::WlRegistry, ()> for RegistryState {
    fn event(
        state: &mut Self,
        _: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            interface, version, ..
        } = event
        {
            state.0.insert(interface, version);
        }
    }
}

fn globals() -> Result<BTreeMap<String, u32>> {
    let connection = Connection::connect_to_env().context("connect to compositor")?;
    let mut queue = connection.new_event_queue::<RegistryState>();
    let _registry = connection.display().get_registry(&queue.handle(), ());
    let mut state = RegistryState::default();
    queue
        .roundtrip(&mut state)
        .context("query compositor registry")?;
    Ok(state.0)
}

#[cfg(test)]
mod tests {
    use super::Capabilities;

    #[test]
    fn hyprland_can_offer_input_without_a_remote_desktop_portal() {
        let mut report = Capabilities::default();
        report
            .globals
            .insert("zwlr_virtual_pointer_manager_v1".into(), 2);
        assert_eq!(report.input_route(), "No complete input route advertised");
        report
            .globals
            .insert("zwp_virtual_keyboard_manager_v1".into(), 1);
        assert_eq!(
            report.input_route(),
            "Wayland virtual pointer + keyboard advertised"
        );
        report.remote_desktop_version = Some(2);
        assert_eq!(report.input_route(), "RemoteDesktop portal advertised");
    }
}
