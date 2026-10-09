# Remote backends: what the tests prove

Companion to `remote-backends.md`. It records a mutation pass over
`kara-vfs` (`RemotePath`, `Location`, `MemoryBackend`) and `kara-fs`
(`LocalBackend`, the fd-based delete, `transfer::move_to`), and lists what this
environment cannot test.

## Method

Each mutation breaks one invariant on purpose, with one edit to a production
source file. Then the relevant tests run
(`cargo test -p kara-fs --test <name> …`, or `cargo test -p kara-vfs --tests`)
and the edit is reverted with `git checkout -- <file>`. A mutation is
**caught** when a test fails that is not on the baseline list (the failures
that already happen without a tmpfs on /tmp, plus the two guards `cb_30` and
`cb_48`). A mutation that **survives** points at a missing or weak test. The
fix is a new test file, and the mutation is run again to confirm that the new
test now fails.

36 mutations were run: 30 were caught at once and 6 survived. All 6 are caught
now.

## Results

`fs/` is `crates/kara-fs/src/`, `vfs/` is `crates/kara-vfs/src/`. When a
mutation broke many tests, only a few of them are named.

| # | Invariant | Mutation | Caught by |
|---|---|---|---|
| F01 | `finish` never makes a half-written file visible | `fs/backend.rs` `temp_name` returns the final name | `cb_14_begin_write_creates_a_hidden_sibling_temp_and_never_the_final_name`, conformance (32 tests) |
| F02 | `abort` leaves no temporary | `abort` skips the `unlinkat` | `cb_18_abort_leaves_the_directory_as_it_was`, conformance |
| F03 | dropping a session leaves no temporary | `Drop` does nothing | `cb_18_dropping_a_session_leaves_the_directory_as_it_was` |
| F04 | a failed `finish` removes its temporary | `finish` marks it done instead of discarding | `cb_16_the_loser_of_a_no_replace_race_gets_already_exists` |
| F05 | `rename` never overwrites, even in a race | `rename_noreplace` replaced by plain `rename` (the existence check stays) | **survived** → `rename_noreplace_race.rs` |
| F06 | `finish` with `replace=false` never clobbers a name taken after `begin_write` | commit always uses `renameat` | `cb_16_the_loser_of_a_no_replace_race_gets_already_exists` |
| F07 | the no-replace primitive is atomic | `fs/trash/mod.rs` `RENAME_NOREPLACE` → no flags | `cb_16_a_broken_symlink_appearing_at_the_target_wins_over_no_replace` |
| F08 | `remove` of a link removes the link | `lstat` → `stat` | `cb_23_remove_of_a_link_to_a_dir_removes_only_the_link` |
| F09 | `remove_tree` never follows a link swapped in after the lstat | `fs/trash/remove.rs` drops `O_NOFOLLOW` | **survived** → `local_backend_mutation_gaps.rs` (RENAME_EXCHANGE race) |
| F10 | `remove_tree` never follows links | drops `O_NOFOLLOW` and `AT_SYMLINK_NOFOLLOW` | `cb_24_remove_tree_unlinks_symlinks_and_never_follows_them`, `delete_permanently_never_follows_a_link_either` |
| F11 | the fd-based delete refuses to cross a device | `crosses_device` → `false` | `a_nested_mount_point_is_refused_and_never_entered` |
| F12 | `error.path` names the caller's path (the refused mount, as a remote path) | `trash_error` always names the argument | `a_nested_mount_point_is_refused_and_never_entered` |
| F13 | a read-only file is not replaced | `refuse_read_only` never refuses | `a_read_only_file_is_not_replaced` |
| F14 | `open_read` opens regular files only | no `is_file` check | `cb_12_open_read_of_a_fifo_is_unsupported_and_does_not_block`, `open_read_of_a_device_is_unsupported` |
| F15 | cancel stops a `remove_tree` walk | `CancelObserver` never cancels | **survived** → `local_backend_mutation_gaps.rs` |
| F16 | a cancel during `list` never returns `Ok` | per-entry cancel check removed | **survived** → `local_backend_mutation_gaps.rs` |
| F17 | a directory is never renamed into its own subtree | subtree check → `false` | `cb_21_rename_into_its_own_subtree_is_invalid_input_naming_to` |
| F18 | a move never deletes what it failed to copy | `fs/transfer.rs` `move_to` ignores the copy error and deletes | **survived** → `move_across_devices.rs` |
| F19 | `finish` fsyncs before the rename | `sync_all` removed | `cb_16_finish_syncs_the_data_before_the_rename` (a **source scan**, not a behaviour test; see below) |
| V01 | siblings that share a prefix are not inside each other | `vfs/path.rs` `starts_with` compares bytes | `a_sibling_sharing_a_prefix_is_not_inside`, `cb_04_starts_with_compares_whole_segments` |
| V02 | faults honour `after` | `take_fault` ignores `after` | `cb_20_a_list_fault_past_the_end_never_fires`, `cb_38_cancel_midway_stops_after_exactly_the_processed_objects` |
| V03 | capacity: a full drive accepts its last byte | `>` → `>=` | `replacing_counts_the_rest_of_the_drive_exactly` |
| V04 | capacity: a replace counts the replaced object once | `saturating_sub(replaced)` → `0` | `replacing_an_object_with_one_of_the_same_size_fits_a_full_drive` |
| V05 | capacity: aborted or dropped sessions give their bytes back | `close` keeps `session_bytes` | `failed_aborted_and_dropped_sessions_give_their_bytes_back` |
| V06 | disconnecting kills open sessions and readers | epoch not bumped | `disconnect_kills_open_sessions_and_readers` |
| V07 | only the canonical URI parses | `canonical` accepts anything | `a_parsed_uri_is_the_canonical_one`, `non_canonical_forms_are_refused` |
| V08 | the URI round trip is lossless | `%` not escaped | `cb_07_round_trip_is_lossless`, `every_location_round_trips_through_its_uri` |
| V09 | `RemotePath` refuses `.` and `..` | dot-segment check removed | `cb_02_dot_segments_are_rejected_not_resolved` |
| V10 | `error.path` names the path the caller used, not the path behind the link | `named` is the identity | `errors_under_a_link_name_the_used_path` |
| V11 | `abort` commits nothing | `abort` commits first | `cb_25_abort_leaves_nothing_and_closes_the_session` |
| V12 | memory `rename` never overwrites | existence check removed | `cb_33_rename_never_overwrites_anything` |
| V13 | cancel stops a memory `remove_tree` midway | per-object token check removed | **survived** → `memory_cancel_midway.rs` |
| V14 | `join` appends exactly one segment | `/` allowed in a segment | `cb_04_join_rejects_anything_that_is_not_one_segment` |
| V15 | `finish` with `replace=false` re-checks the target | check removed from commit | `cb_23_replace_false_is_checked_again_at_finish` |
| V16 | a disconnected backend answers `Unavailable` | `ensure_connected` always succeeds | `cb_27_disconnect_reaches_every_method_with_the_argument_path` |
| V17 | memory: no rename into its own subtree | subtree check → `false` | `cb_35_rename_into_own_subtree_is_refused` |

Why each survivor slipped through:

- **F05:** the existence check right before the rename hides the missing
  atomicity. The new test makes two renames race onto one name for 3 s; under
  the mutation both win in round 1.
- **F09:** the lstat that runs before the open filters out links, except
  when a link arrives between the two calls. The existing swap test
  (rename + symlink) almost never hit that gap. `RENAME_EXCHANGE` swaps a
  directory and a link atomically in a tight loop, which hits it within about
  30 rounds.
- **F15, F16, V13:** the existing cancel races accepted either outcome, so a
  cancel that was never checked mid-walk still passed. The new tests cancel
  1–8 ms into a job that takes far longer (50 000 entries, 20 000 files,
  100 000 objects) and require `Cancelled`.
- **F18:** nothing exercised a move across devices. The new test copies to a
  tmpfs (/tmp on CI, /dev/shm here, or a tmpfs it mounts) and makes the copy
  fail part-way.

The new tests passed five runs in a row on the real code.

## Not tested here

- **fsync durability.** `finish` calls `sync_all` on the temporary and fsyncs
  the directory after the rename. Proving that this survives a power cut needs
  crash injection (dm-flakey, or a VM that is killed). F19 is caught only
  because a test scans the source for the call. No test checks the directory
  fsync at all.
- **The `RENAME_NOREPLACE` fallback** (link-then-unlink, then
  check-then-rename) runs only when the kernel or filesystem returns
  `EINVAL`/`ENOSYS`/`EOPNOTSUPP` (some NFS, FUSE, older kernels). ext4 and
  tmpfs here support the flag, so the fallback never runs. Testing it needs an
  NFS or FUSE mount, or a seam to inject the errno.
- **Permission checks.** The tests run as root, so `chmod 000` stops nothing:
  `an_unreadable_subdirectory_is_reported_by_its_remote_path` skips itself, and
  the `ACCESS`/`PERM` branch of `rename_failure` (which names the source when
  its directory is not writable) never runs. Part of the baseline failures are
  permission tests for the same reason.
- **Volume trash.** `/tmp` is not a tmpfs here, so the `trash_volume` tests
  (which need /tmp on another device than the tree) are in the baseline
  failures. Mounting a tmpfs works here (root), and the mount-point and
  cross-device tests use one, but the trash suite expects the CI layout.
- **Races in `rename_failure` and `same_entry`.** A source that disappears
  between the lstat and the rename (`ENOENT`, which names `from`) cannot be
  reproduced reliably.

## Two review items that stay as they are

Both are pinned by `cb_07_list_entries_are_exactly_what_describe_returns` in
`crates/kara-fs/tests/local_backend_read.rs`, a protected (hash-checked) file:

- **Size of a symlink to a regular file.** `list` and `stat` report the link's
  own size, not the target's: the test requires every entry to equal
  `describe()`, which reads the size from `lstat`.
- **`FileEntry.location` under `with_root`.** It is still the local parent
  directory, which includes the root. The same test asserts
  `entry.location == Some(<local dir>)`.

Changing either one means changing that contract first.
