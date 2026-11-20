fn main() {
    #[cfg(windows)]
    {
        use std::{env, fs, path::PathBuf};
        let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
        let out = PathBuf::from(env::var("OUT_DIR").expect("out dir"));
        println!("cargo:rerun-if-changed=assets/clarora.ico");
        let icon = root.join("assets/clarora.ico");
        let version = env::var("CARGO_PKG_VERSION").expect("crate version");
        let mut parts = version
            .split('.')
            .take(3)
            .map(|part| part.parse::<u16>().unwrap_or(0));
        let file_version = format!(
            "{},{},{},0",
            parts.next().unwrap_or(0),
            parts.next().unwrap_or(0),
            parts.next().unwrap_or(0)
        );
        let icon = icon.display().to_string().replace('\\', "\\\\");
        let rc = out.join("clarora-resource.rc");
        fs::write(
            &rc,
            format!(
                r#"1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {file_version}
PRODUCTVERSION {file_version}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "Clarora"
      VALUE "FileDescription", "Clarora"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "clarora-gpui"
      VALUE "OriginalFilename", "clarora-gpui.exe"
      VALUE "ProductName", "Clarora"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
            ),
        )
        .expect("write resource file");
        embed_resource::compile(&rc, embed_resource::NONE);
    }
}
