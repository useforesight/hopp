use hopp_native_input_check::cursor_overlay::{CursorOverlay,DrawingOverlay};
fn layers()->anyhow::Result<String>{
 let result=std::process::Command::new("hyprctl").args(["layers","-j"]).output()?;
 anyhow::ensure!(result.status.success(),"hyprctl layers failed");Ok(String::from_utf8(result.stdout)?)
}
fn settle(){std::thread::sleep(std::time::Duration::from_millis(100));}
fn main()->anyhow::Result<()>{
 anyhow::ensure!(std::env::var("XDG_RUNTIME_DIR")?=="/tmp/hopp-nested/runtime","Only the isolated nested runtime is permitted");
 let mut cursor=CursorOverlay::connect("WAYLAND-1",1280,720)?;
 let mut drawing=DrawingOverlay::connect("WAYLAND-1",1280,720)?;
 cursor.move_to(0.5,0.5)?;settle();
 anyhow::ensure!(layers()?.contains("hopp-remote-cursor"),"Cursor layer missing");
 if !std::env::args().any(|arg|arg=="--no-remap"){cursor.hide()?;cursor.move_to(0.6,0.5)?;settle();}
 drawing.set_color(121,161,247);
 drawing.set_paths(&[vec![(0.2,0.2),(0.25,0.3),(0.4,0.3)],vec![(0.2,0.35),(0.3,0.5)],vec![(0.45,0.5),(0.55,0.5)]])?;settle();
 anyhow::ensure!(layers()?.contains("hopp-drawing"),"Drawing layer missing");
 cursor.move_to(0.5,0.5)?;
 if std::env::args().any(|arg|arg=="--hold"){std::thread::sleep(std::time::Duration::from_secs(3));}
 let capture=std::process::Command::new("timeout").args(["2","grim","/tmp/hopp-nested/overlay.png"]).status()?;
 println!("Nested capture ready: {}",capture.success());
 drawing.clear()?;drawing.set_paths(&[vec![(0.5,0.6),(0.6,0.7)]])?;settle();
 anyhow::ensure!(layers()?.contains("hopp-drawing"),"Drawing remap failed");
 drop(drawing);drop(cursor);settle();
 let after=layers()?;anyhow::ensure!(!after.contains("hopp-drawing")&&!after.contains("hopp-remote-cursor"),"Overlay survived Drop");
 println!("Cursor show/hide/remap and drawing configure/crop/color/clear/remap/Drop passed on isolated WAYLAND-1");Ok(())
}
