/// Operating system name as reported to viewers.
pub(crate) fn platform() -> String {
    std::env::consts::OS.to_owned()
}

/// Human-readable device name: the phone model on Android, the hostname
/// elsewhere.
pub(crate) fn device() -> String {
    detect().unwrap_or_else(|| "unknown".to_owned())
}

#[cfg(target_os = "android")]
#[allow(unsafe_code)]
fn detect() -> Option<String> {
    let mut model = [0u8; 92]; // PROP_VALUE_MAX
    // SAFETY: the name is a NUL-terminated literal and `model` has room for
    // PROP_VALUE_MAX bytes, the most `__system_property_get` ever writes.
    let len = unsafe {
        libc::__system_property_get(c"ro.product.model".as_ptr(), model.as_mut_ptr().cast())
    };
    let len = usize::try_from(len).ok().filter(|l| *l > 0)?;
    String::from_utf8(model[..len.min(model.len())].to_vec()).ok()
}

#[cfg(not(target_os = "android"))]
fn detect() -> Option<String> {
    ["HOSTNAME", "COMPUTERNAME"]
        .iter()
        .find_map(|key| std::env::var(key).ok().filter(|v| !v.is_empty()))
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
        })
}
