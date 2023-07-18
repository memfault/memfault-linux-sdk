//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Recovery of old log files after restarting the memfaultd service.
//!
use std::fs;
use std::fs::remove_file;
use std::iter::{once, zip};
use std::mem::take;
use std::path::{Path, PathBuf};

use crate::logs::completed_log::CompletedLog;
use eyre::Result;
use log::{debug, warn};
use uuid::Uuid;

use crate::mar::CompressionAlgorithm;
use crate::util::fs::get_files_sorted_by_mtime;
use crate::util::path::file_prefix;

struct FileInfo {
    path: PathBuf,
    uuid: Option<Uuid>,
    size: Option<u64>,
}

#[derive(Debug, PartialEq)]
struct LogFileToRecover {
    path: PathBuf,
    cid: Uuid,
    next_cid: Uuid,
}

#[derive(Debug, PartialEq)]
struct Recovery {
    to_delete: Vec<PathBuf>,
    to_recover: Vec<LogFileToRecover>,
    next_cid: Uuid,
}

fn should_recover(file_info: &FileInfo) -> bool {
    match file_info {
        FileInfo { uuid: None, .. } => false,
        FileInfo { size: None, .. } => false,
        FileInfo {
            size: Some(size), ..
        } if *size == 0 => false,
        // Only keep files that are not empty and have a valid uuid:
        _ => true,
    }
}

/// The "functional core" of the recovery logic. It is pure for unit-testing sake.
/// Note: the file_infos must be sorted by mtime, newest last.
fn get_recovery(file_infos: Vec<FileInfo>, gen_uuid: fn() -> Uuid) -> Recovery {
    // If the last file was empty we'll delete it, but we want to reuse the CID because it's
    // possible the previous file is already uploaded and references that CID:
    let last_cid = file_infos
        .iter()
        .filter_map(|info| match info {
            FileInfo {
                uuid: Some(uuid), ..
            } => match should_recover(info) {
                true => Some(gen_uuid()),
                false => Some(*uuid),
            },
            _ => None,
        })
        .last()
        .unwrap_or_else(gen_uuid);

    let (mut to_recover_infos, to_delete_infos): (Vec<FileInfo>, Vec<FileInfo>) =
        file_infos.into_iter().partition(should_recover);

    #[allow(clippy::needless_collect)]
    let next_cids: Vec<Uuid> = to_recover_infos
        .iter()
        .skip(1)
        .map(|i| i.uuid.unwrap())
        .chain(once(last_cid))
        .collect();

    Recovery {
        to_delete: to_delete_infos.into_iter().map(|info| info.path).collect(),
        to_recover: zip(to_recover_infos.iter_mut(), next_cids.into_iter())
            .map(|(info, next_cid)| LogFileToRecover {
                path: take(&mut info.path),
                cid: info.uuid.unwrap(),
                next_cid,
            })
            .collect(),
        next_cid: last_cid,
    }
}

pub fn recover_old_logs<R: FnMut(CompletedLog) -> Result<()> + Send + 'static>(
    tmp_logs: &Path,
    on_log_recovery: &mut R,
) -> Result<Uuid> {
    // Make a list of all the info of the files we want to collect, parsing the CID from the
    // filename and getting the size on disk:
    let file_infos = get_files_sorted_by_mtime(tmp_logs)?
        .into_iter()
        .map(|path| {
            let uuid = file_prefix(&path)
                .and_then(|prefix| Uuid::parse_str(&prefix.to_string_lossy()).ok());
            let size = fs::metadata(&path).ok().map(|metadata| metadata.len());
            FileInfo { path, uuid, size }
        })
        .collect();

    let Recovery {
        next_cid,
        to_delete,
        to_recover,
    } = get_recovery(file_infos, Uuid::new_v4);

    // Delete any unwanted files from disk:
    for path in to_delete {
        if let Err(e) = remove_file(&path) {
            warn!(
                "Unable to delete bogus log file: {} - {}.",
                path.display(),
                e
            );
        }
    }

    for LogFileToRecover {
        path,
        cid,
        next_cid,
    } in to_recover
    {
        // Write the MAR entry which will move the logfile
        debug!("Recovering logfile: {:?}", path.display());

        if let Err(e) = (on_log_recovery)(CompletedLog {
            path,
            cid,
            next_cid,
            compression: CompressionAlgorithm::Zlib,
        }) {
            warn!("Unable to recover log file: {}", e);
        }
    }

    Ok(next_cid)
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID_A: Uuid = Uuid::from_u128(1);
    const UUID_B: Uuid = Uuid::from_u128(2);
    const UUID_NEW: Uuid = Uuid::from_u128(3);

    const PATH_A: &str = "/tmp_log/11111111-1111-1111-1111-111111111111";
    const PATH_B: &str = "/tmp_log/22222222-2222-2222-2222-222222222222";

    #[test]
    fn empty_logging_directory() {
        let file_infos: Vec<FileInfo> = vec![];
        let expected = Recovery {
            to_delete: vec![],
            to_recover: vec![],
            next_cid: UUID_NEW,
        };
        assert_eq!(get_recovery(file_infos, gen_uuid), expected);
    }

    #[test]
    fn delete_improperly_named_files() {
        // File w/o proper UUID name should get deleted, even if it's not empty:
        let file_infos = vec![FileInfo {
            path: PathBuf::from("/tmp_log/foo"),
            uuid: None,
            size: Some(1),
        }];
        let expected = Recovery {
            to_delete: vec![PathBuf::from("/tmp_log/foo")],
            to_recover: vec![],
            next_cid: UUID_NEW,
        };
        assert_eq!(get_recovery(file_infos, gen_uuid), expected);
    }

    #[test]
    fn use_empty_trailing_uuid_named_file_as_next_cid() {
        // Any trailing but empty UUID-named file should be used as next_cid:
        let file_infos = vec![
            FileInfo {
                path: PATH_A.into(),
                uuid: Some(UUID_A),
                size: Some(1),
            },
            FileInfo {
                path: PATH_B.into(),
                uuid: Some(UUID_B),
                size: Some(0),
            },
        ];
        let expected = Recovery {
            to_delete: vec![PATH_B.into()],
            to_recover: vec![LogFileToRecover {
                path: PATH_A.into(),
                cid: UUID_A,
                next_cid: UUID_B,
            }],
            next_cid: UUID_B,
        };
        assert_eq!(get_recovery(file_infos, gen_uuid), expected);
    }

    #[test]
    fn dont_use_non_trailing_empty_uuid_named_file_as_next_cid() {
        // An empty UUID-named file that is not trailing (has newer, non-empty files following)
        // should not be used as next_cid:
        let file_infos = vec![
            FileInfo {
                path: PATH_A.into(),
                uuid: Some(UUID_A),
                size: Some(0),
            },
            FileInfo {
                path: PATH_B.into(),
                uuid: Some(UUID_B),
                size: Some(1),
            },
        ];
        let expected = Recovery {
            to_delete: vec![PATH_A.into()],
            to_recover: vec![LogFileToRecover {
                path: PATH_B.into(),
                cid: UUID_B,
                next_cid: UUID_NEW,
            }],
            next_cid: UUID_NEW,
        };
        assert_eq!(get_recovery(file_infos, gen_uuid), expected);
    }

    #[test]
    fn chain_cids() {
        // Chain CIDs, setting next_cid to the CID of the next file or a newly generated one if it's the
        // last file (and it's not empty):
        let file_infos = vec![
            FileInfo {
                path: PATH_A.into(),
                uuid: Some(UUID_A),
                size: Some(1),
            },
            FileInfo {
                path: PATH_B.into(),
                uuid: Some(UUID_B),
                size: Some(1),
            },
        ];
        let expected = Recovery {
            to_delete: vec![],
            to_recover: vec![
                LogFileToRecover {
                    path: PATH_A.into(),
                    cid: UUID_A,
                    next_cid: UUID_B,
                },
                LogFileToRecover {
                    path: PATH_B.into(),
                    cid: UUID_B,
                    next_cid: UUID_NEW,
                },
            ],
            next_cid: UUID_NEW,
        };
        assert_eq!(get_recovery(file_infos, gen_uuid), expected);
    }

    fn gen_uuid() -> Uuid {
        UUID_NEW
    }
}
