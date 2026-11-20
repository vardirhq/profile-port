//! Platform-specific details: metadata capture and error classification.

use std::fs::Metadata;
use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

use profileport_package::{Meta, SkipReason, Timestamp};

pub(crate) fn timestamp(t: io::Result<SystemTime>) -> Option<Timestamp> {
    let t = t.ok()?;
    Some(match t.duration_since(UNIX_EPOCH) {
        Ok(d) => Timestamp { secs: d.as_secs() as i64, nanos: d.subsec_nanos() },
        Err(e) => {
            let d = e.duration();
            // Before 1970: represent as negative seconds + positive nanos.
            let (secs, nanos) = if d.subsec_nanos() == 0 {
                (-(d.as_secs() as i64), 0)
            } else {
                (-(d.as_secs() as i64) - 1, 1_000_000_000 - d.subsec_nanos())
            };
            Timestamp { secs, nanos }
        }
    })
}

pub(crate) fn to_system_time(t: &Timestamp) -> Option<SystemTime> {
    let base = if t.secs >= 0 {
        UNIX_EPOCH.checked_add(std::time::Duration::from_secs(t.secs as u64))?
    } else {
        UNIX_EPOCH.checked_sub(std::time::Duration::from_secs(t.secs.unsigned_abs()))?
    };
    base.checked_add(std::time::Duration::from_nanos(t.nanos as u64))
}

pub(crate) fn meta_of(m: &Metadata) -> Meta {
    Meta {
        modified: timestamp(m.modified()),
        created: timestamp(m.created()),
        readonly: m.permissions().readonly(),
        win_attributes: win_attributes(m),
    }
}

#[cfg(windows)]
fn win_attributes(m: &Metadata) -> Option<u32> {
    use std::os::windows::fs::MetadataExt;
    Some(m.file_attributes())
}

#[cfg(not(windows))]
fn win_attributes(_: &Metadata) -> Option<u32> {
    None
}

/// OneDrive "Files On-Demand" placeholders whose content is in the cloud.
/// Reading them would trigger a download (or fail offline), so they are
/// skipped and reported instead.
#[cfg(windows)]
pub(crate) fn is_cloud_only(m: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x0004_0000;
    const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
    m.file_attributes() & (FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS) != 0
}

#[cfg(not(windows))]
pub(crate) fn is_cloud_only(_: &Metadata) -> bool {
    false
}

/// Map an error opening or reading a source file to a report category.
pub(crate) fn classify(e: &io::Error) -> SkipReason {
    #[cfg(windows)]
    {
        const ERROR_SHARING_VIOLATION: i32 = 32;
        const ERROR_LOCK_VIOLATION: i32 = 33;
        if matches!(e.raw_os_error(), Some(ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)) {
            return SkipReason::Locked;
        }
    }
    match e.kind() {
        io::ErrorKind::PermissionDenied => SkipReason::PermissionDenied,
        io::ErrorKind::NotFound => SkipReason::Vanished,
        _ => SkipReason::ReadError,
    }
}

/// Size and modification time, used to detect a file changing while it is
/// being copied.
pub(crate) fn fingerprint(m: &Metadata) -> (u64, Option<SystemTime>) {
    (m.len(), m.modified().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_roundtrip_including_pre_1970() {
        for secs in [-86_400i64 * 365 * 30, -1, 0, 1, 1_700_000_000] {
            for nanos in [0u32, 1, 999_999_999] {
                let t = Timestamp { secs, nanos };
                assert_eq!(timestamp(Ok(to_system_time(&t).unwrap())).unwrap(), t);
            }
        }
    }
}
