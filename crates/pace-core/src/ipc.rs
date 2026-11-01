// SPDX-License-Identifier: GPL-3.0-only
//! Names of the local D-Bus interface between the recorder and its clients.

pub const BUS_NAME: &str = "io.github.x_vlad_x.PaceBeads.Recorder";
pub const OBJECT_PATH: &str = "/io/github/x_vlad_x/PaceBeads/Recorder";
pub const INTERFACE: &str = "io.github.x_vlad_x.PaceBeads.Recorder";

pub const INTROSPECTION_XML: &str = r#"<node>
  <interface name="io.github.x_vlad_x.PaceBeads.Recorder">
    <method name="GetState">
      <arg type="t" name="today_steps" direction="out"/>
      <arg type="s" name="pause" direction="out"/>
      <arg type="b" name="sensor_connected" direction="out"/>
    </method>
    <method name="Pause">
      <arg type="u" name="minutes" direction="in"/>
    </method>
    <method name="Resume"/>
    <method name="SetTodaySteps">
      <arg type="t" name="steps" direction="in"/>
    </method>
    <method name="Reload"/>
    <signal name="StateChanged">
      <arg type="t" name="today_steps"/>
      <arg type="s" name="pause"/>
      <arg type="b" name="sensor_connected"/>
    </signal>
  </interface>
</node>"#;

/// Translation directory next to the running program's installation prefix.
pub fn locale_directory() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| Some(path.parent()?.parent()?.join("share/locale")))
        .unwrap_or_else(|| "/usr/share/locale".into())
}
