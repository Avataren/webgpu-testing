//! Launch browser automation without npm dependencies or persistent browser state.
use crate::{Output, Server};
use serde_json::Value;
use std::{
    io::Write,
    process::{Command, Stdio},
};

impl Server {
    pub(crate) fn web_playtest(&mut self, args: &Value) -> Result<Output, String> {
        let mut args = args.clone();
        let directory = self
            .engine
            .repo
            .join(args["directory"].as_str().ok_or("directory required")?);
        if !directory.join("index.html").is_file() {
            return Err("directory has no index.html; run web_build first".into());
        }
        args["directory"] = Value::String(directory.to_string_lossy().into_owned());
        if let Some(save) = args["save_path"].as_str() {
            args["save_path"] =
                Value::String(self.engine.repo.join(save).to_string_lossy().into_owned());
        }
        let mut child = Command::new("node")
            .args(["--input-type=module", "-"])
            .arg(serde_json::to_string(&args).map_err(|e| e.to_string())?)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot start Node: {e}"))?;
        child
            .stdin
            .take()
            .ok_or("Node stdin unavailable")?
            .write_all(include_str!("../web_playtest.mjs").as_bytes())
            .map_err(|e| e.to_string())?;
        let output = child.wait_with_output().map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "browser playtest failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        let mut caption: Value = serde_json::from_slice(&output.stdout)
            .map_err(|e| format!("invalid browser report: {e}"))?;
        let png_base64 = caption
            .as_object_mut()
            .and_then(|v| v.remove("png_base64"))
            .and_then(|v| v.as_str().map(str::to_owned))
            .ok_or("browser returned no screenshot")?;
        Ok(Output::Image {
            png_base64,
            caption,
        })
    }
}
