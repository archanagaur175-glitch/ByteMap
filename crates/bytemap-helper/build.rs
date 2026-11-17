// P6.T1: embed the requireAdministrator manifest so helper.exe can only run
// elevated (fail-closed: the main app never runs as administrator; owner
// approved the embed-resource build dependency for this on 2026-11-17).
fn main() {
    if cfg!(target_os = "windows") {
        embed_resource::compile("helper.rc", embed_resource::NONE)
            .manifest_optional()
            .unwrap();
    }
}
