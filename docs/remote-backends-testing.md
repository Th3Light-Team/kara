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

## `kara-ops` over Locations (step 3)

Same method, over `crates/kara-ops/src/` (`R` is `runner/remote.rs`, `RU`
`runner.rs`, `L` `location.rs`, `U` `undo.rs`, `RD` `remote_undo.rs`), running
all of `cargo test -p kara-ops` for each mutation. The remote drive is
`MemoryBackend` (both profiles) with fault injection; the tests wrap it to
spy on calls (`Spy`, which also logs `finish`/`abort`/drop of each session),
to hold a reader mid-copy (`Gated`), to lie about sizes (`LyingStat`), to hide
a name from `stat` (`Blind`) or to break the contract on purpose
(`Defective`). A crashed test binary counts as caught (M17).

29 mutations: 24 caught at once, 3 survived and are caught now, 2 are
equivalent.

| # | Invariant | Mutation | Caught by |
|---|---|---|---|
| M01 | the source is never deleted after a failed copy | `R` `loc_leaf` ignores `copied.is_none()` | `a_failure_mid_copy_leaves_no_destination_and_keeps_the_source_of_a_move`, `one_failing_child_keeps_the_whole_source_tree_of_a_move` (+3) |
| M02 | the size of the copy is verified | `R` `verify` always `Ok` | `a_size_mismatch_keeps_the_source_and_removes_the_bad_copy`, `…_after_a_server_side_copy_keeps_the_source_too` |
| M03 | a failed or cancelled session is aborted, not just dropped | `abort()` → `drop(session)` | `a_failed_copy_aborts_its_session`, `cancelling_mid_copy_aborts_the_session_and_keeps_the_source` |
| M04 | an unconfirmed permanent delete is refused | `RU` confirmation check → `false` | `an_unconfirmed_delete_is_refused_before_anything_is_touched` |
| M05 | `Unavailable` → `MediaGone` | `L` → `Other` | `every_backend_error_kind_maps_onto_the_failure_kinds`, `a_lost_connection_is_media_gone_and_a_blanket_retry_does_not_loop` (+2) |
| M06 | a conflict is never replaced silently | `begin_write(…, replace)` → `true` | `a_name_taken_behind_the_conflict_check_is_never_overwritten` |
| M07 | server-side copy is used when declared | `server_side` → `false` | `a_copy_inside_a_drive_with_server_side_copy_moves_no_bytes`, `a_move_inside_an_object_store_is_copy_verify_delete` |
| M08 | a move without `undo_move` is recorded as not undoable | `move_record` capability check → `true` | `a_move_on_a_drive_without_undo_move_is_recorded_and_disabled_with_a_reason` (+1) |
| M09 | undo re-checks the capability | `RD` `rename_back` ignores `allowed` | `a_recorded_remote_move_is_not_undone_if_the_drive_no_longer_allows_it` |
| M10 | a move between backends is not undoable | cross-backend check → `false` | `a_move_across_backends_and_a_replacing_copy_are_never_undoable` (+1) |
| M11 | an all-local request runs the old code | `RU` `to_local()` routing skipped | **equivalent**: the location worker hands local → local items to the old `node()` itself (see M11b, M11c) |
| M11b | a local item in a mixed job runs the old code | `R` local → local delegation removed | **survived** (LocalBackend's `rename` is rename(2) too, so the inode test cannot tell) → `local_copies_inside_any_location_job_keep_mode_and_mtime` |
| M11c | both of the above | M11 + M11b | `every_local_scenario_gives_the_same_outcome_through_both_apis`, `a_local_folder_moved_on_one_volume_is_renamed_not_copied`, `local_copies_…_keep_mode_and_mtime` |
| M12 | the job's cancel reaches `remove_tree` | fresh `Cancel` passed | `cancelling_a_remote_delete_reaches_remove_tree_through_its_token` |
| M13 | a folder move removes only what it copied | `remove_tree` on the source instead | `a_file_that_appears_in_the_source_during_a_folder_move_is_not_deleted` (+2) |
| M14 | remote Replace is file over file only | the `FileOverFile` check → `true` | `replace_never_destroys_a_remote_folder_or_puts_a_folder_over_a_file` |
| M15 | cancel is checked at every chunk | check removed from the loop | `cancelling_mid_copy_aborts_the_session_and_keeps_the_source`, `a_paused_copy_…` |
| M16 | a retry does not count bytes twice | counter not reset | `retry_after_a_transient_failure_copies_once_and_counts_bytes_once` |
| M17 | a folder is not copied into itself | into-itself check → `false` | `copying_a_remote_folder_into_itself_is_refused` (stack overflow: the binary aborts) |
| M18 | a not-undoable record disables «Deshacer» | `can_undo` → `!is_empty()` | `a_move_on_a_drive_without_undo_move_…`, `a_move_across_backends_…` |
| M19 | a copy that replaced a remote file is not undoable | replaced guard → `false` | `replace_on_a_remote_file_writes_with_replace_and_is_not_undoable` (+1) |
| M20 | «Calculando…» measures remote trees | `measure` does not recurse | `a_remote_tree_is_copied_to_a_local_folder_and_measured_first`, `a_confirmed_remote_delete_…` |
| M21 | a mismatched fresh copy is removed | removal skipped | both size-mismatch tests |
| M22 | pause holds at a chunk boundary | `wait_while_paused` removed | `a_paused_copy_waits_at_a_chunk_boundary_and_a_cancel_still_works` |
| M23 | a listing with errors is not treated as complete | `listing.errors` ignored | **survived** → `a_folder_listed_with_errors_is_reported_and_its_move_keeps_every_source` |
| M24 | a source that cannot be removed marks the folder partial | the `whole = Partial` after a failed removal dropped | **equivalent**: the only consequence is the final `remove` of the folder, which a backend refuses while the file is still in it. The failure is still reported (`a_source_that_cannot_be_removed_after_a_folder_move_is_reported_and_kept`) |
| M25 | undo never overwrites what took the old place | `ensure_free` removed | **survived** (MemoryBackend's `rename` refuses anyway) → `undo_checks_the_old_place_itself_instead_of_trusting_the_backend`, with a backend whose `rename` overwrites |
| M26 | undo of a new remote folder removes it only while empty | `remove` → `remove_tree` | `undo_of_a_remote_new_folder_removes_it_only_while_empty` |
| M27 | children of a folder moved by copying are not removed one by one | per-file removal regardless of `move_now` | `one_failing_child_keeps_the_whole_source_tree_of_a_move`, `a_symlink_is_not_turned_into_a_copy_of_its_target` |

Why the survivors slipped through:

- **M11b:** both paths move by rename(2), so the inode stays the same either
  way. A copy is what tells them apart: only the old path keeps mode and mtime.
- **M23:** `MemoryBackend` never returns a partial listing. `Defective` hides
  one entry and reports an error for it, which is what a real backend does
  with an entry it cannot read.
- **M25:** the conformance suite guarantees that `rename` never overwrites, so
  the check in `kara-ops` is only defence in depth. The test uses a backend
  that breaks that rule on purpose.

Also found in review before the pass (not a mutation): the end of a folder
move across backends used `remove_tree` on the source, which deleted a file
written into the source after the folder was listed. Fixed (only the copied
sources are removed); M13 is the regression check.

Not tested here: the bridge and QML (no Qt), real SFTP/S3 latency and partial
reads, and a backend whose `stat` reports no size (verification then fails and
the source stays, by design, but no backend in the tree does that).

## kara-remote (registry, config, secrets)

9 mutations on `registry.rs` and `config.rs`; 7 caught at once, 1 survived and is now
caught, 1 is equivalent (`backend()` also checks the state, but `set_state` already
drops the backend of a non-`Ready` drive).

| Invariant | Mutation | Caught by |
|---|---|---|
| A secret is stored only after a connection with it worked and only if asked | store without `remember` | `a_secret_is_not_kept_unless_the_user_asked`, `a_wrong_secret_is_not_remembered` |
| Only a lost connection marks the drive lost | any error marks it lost | `a_lost_connection_stops_the_drive_resolving` |
| One connect at a time per drive | busy check removed | `a_second_connect_while_connecting_is_refused` |
| Removing a drive deletes its secret | secret kept | `removing_a_drive_deletes_its_secret` |
| Secret prompts are bounded | 3 attempts → 1 | `a_secret_is_asked_for_and_remembered_after_it_worked` (needs the retry path) |
| A cancelled connect never reaches the factory | cancel check removed | **survived** (MemoryFactory checks the token itself) → `registry_cancel::a_cancelled_connect_never_calls_the_factory` |
| No secret-looking parameter reaches settings | guard disabled | `secret_looking_parameters_are_refused`, `nothing_secret_reaches_the_file` |
| Re-storing a drive drops parameters that were removed | `remove_section` skipped | `storing_replaces_removed_parameters` |
| A lost drive does not resolve | state check in `backend()` removed | equivalent mutant |

Not testable here: `KeyringSecretStore` (needs a session bus and an unlocked
keyring; compiled and linted only, see `remote-drives-handoff.md`).

### Groups and import (11 more mutations)

10 caught, 1 equivalent (a `Match` block cannot leak into the next drive because
`finish` already resets the block). Covered: own secret beats group secret, `ForGroup`
stores on the group, removing a drive keeps the group's secret, wildcard/negated hosts
skipped, `ProxyJump` reported, duplicates and bad ports reported, CSV header skipped,
group name validation and persistence.

## SFTP adapter (step 5)

All hermetic: `cargo test -p kara-remote` (the self dev-dependency enables
`sftp`). The server is `tests/support/mod.rs`: `russh`'s server with a
`russh-sftp` handler over a tempdir on `127.0.0.1:<ephemeral>`, answering like
OpenSSH's `sftp-server` (its `errno` table, `readdir` pages of 100 with `.`/`..`
and `lstat` attributes, plain `rename` by link + unlink, optional
`posix-rename`/`fsync`/`hardlink` extensions) with fault injection: delay per
request, stall, «No space left on device» past an offset, `PERMISSION_DENIED`
under a prefix, dropping every connection after N bytes written/read, short
reads, a frozen transport, and a lenient `rename` that replaces.

| File | Tests | What |
|---|---|---|
| `sftp_conformance.rs` | 5 | `conformance::run` + `run_extra`: with and without posix-rename/fsync, strerror messages, under `root`; capabilities follow the probe |
| `sftp_differential_local.rs` | 4 | the kara-fs generator against SftpBackend and LocalBackend (no links, links down, links anywhere without renames, no posix-rename); 40 seeds × 100 ops each by default, **400 seeds passed once** |
| `sftp_auth.rs` | 15 | password ok / wrong (`AuthFailed`) / missing (`AuthRequired`), key file, unknown key, encrypted key with/without/wrong passphrase, `~/` expansion, absent agent, agent key, agent limited to `key_file`, registry password prompt, unreachable, connect timeout, connect cancel, no secret in errors or `Debug` |
| `sftp_agent_env.rs` | 1 | `$SSH_AUTH_SOCK` dangling and live (own file: changes the environment) |
| `sftp_host_keys.rs` | 10 | unknown → prompt → remembered (0600 file, 0700 folder) → silent; append keeps lines and modes; refusal writes nothing; trust once; changed key refused, file untouched (bytes and mtime) even on «trust and remember»; hashed entries; port-22 entry vs `[host]:port`; `@revoked`; prompts through the registry; parser patterns |
| `sftp_failures.rs` | 16 | drop mid-write (no final name), abort/drop remove the temporary, drop mid-read → `Unavailable` naming the file, stall → timeout in < 2.5 s, every call `Unavailable` at once after loss, keepalive, full disk → `NoSpace`, denied → `PermissionDenied` without server paths, errors never name the temporary, read-only not replaced, offset reads, short reads, `remove_tree` and links, cancel of list / `remove_tree`, links listed like LocalBackend |
| `sftp_cancel_in_flight.rs` | 1 | a cancel during one slow `readdir` page |
| `sftp_lenient_rename.rs` | 3 | a server whose plain rename replaces: `rename` and `finish(replace=false)` still refuse; conformance |
| `sftp_with_ops.rs` | 6 | `report_failure` → `Lost` → reconnect → `Ready`; kara-ops copy folder up and down, move up, cancel an upload (no file, no temporary), connection lost mid-move (`MediaGone`, source kept), group secret |
| `sftp_source_rules.rs` | 4 | no `unwrap`/`expect`/panicking macros, no printing/logging in `src/sftp`; `Secret::expose` only in `connect.rs` (twice) |
| `sftp_real_server.rs` | 1 (ignored) | conformance against a real sshd: `KARA_TEST_SFTP="host:port,user,keyfile"` |

The differential test found one bug before the mutation pass: without
posix-rename, replacing a file whose path went through the link being replaced
(`/a/a` with `/a -> .`) removed the link and then could not find the temporary.
`begin_write` now resolves the target directory once (`realpath`).

### Mutation pass

Same method as above, over `crates/kara-remote/src/sftp/` (harness: apply one
edit, run the named test files, `git checkout` + `touch`). 32 mutations: 30
caught at once, 1 survived and is caught now, 1 equivalent.

| # | Invariant | Mutation | Caught by |
|---|---|---|---|
| S01 | `replace=false` never overwrites | `io.rs` no-replace commit uses posix-rename, no existence check | `finish_without_replace_refuses_a_target_that_appeared_meanwhile`, conformance `replace_false_race_at_finish` (4 suites) |
| S02 | `abort` removes the temporary | the `remove` of the temporary skipped | `abort_and_drop_remove_the_temporary_on_the_server`, `a_full_disk_…`, conformance |
| S03 | `Drop` removes the temporary | `Drop` does nothing | `abort_and_drop_…`, conformance `drop_leaves_nothing` |
| S04 | `finish` renames only after every write was acknowledged | in-flight writes dropped instead of awaited | `a_full_disk_is_no_space_and_leaves_nothing_behind`, `errors_never_name_the_temporary_or_the_server_path` |
| S05 | `finish` sends the buffered tail | tail never sent | conformance (content) |
| S06 | `error.path` is the caller's path | a write failure names `/` | `a_connection_dropped_mid_write_…`, `a_full_disk_…`, `errors_never_name_…` |
| S07 | a changed host key is never trusted silently | `Changed` counts as trusted | `a_changed_key_is_refused_by_default_…`, `a_hashed_entry_…` |
| S08 | `known_hosts` is never rewritten for a changed key | append on «trust and remember» of a changed key | `a_changed_key_is_refused_by_default_and_the_file_is_untouched` |
| S09 | a refusal writes nothing | append on `Refuse` | `refusing_an_unknown_key_…`, `a_changed_key_…` |
| S10 | reads start at the offset | reader starts at 0 | `offset_reads_start_exactly_there`, conformance `read_offsets` |
| S11 | a short read is asked again from where it ended | offset not reset after a short read | `a_short_read_from_the_server_does_not_lose_or_repeat_bytes` |
| S12 | `remove_tree` never follows links | child type from `stat` (follows) | `remove_tree_removes_links_and_never_what_they_point_to` |
| S13 | cancel stops a listing during a page | `readdir` waited with a fresh token | **survived** (the per-page check still cancelled) → `sftp_cancel_in_flight.rs` |
| S14 | cancel stops `remove_tree` per entry | per-entry check removed | `cancel_stops_remove_tree_midway` |
| S15 | cancel stops a connect | the connect never watches the token | `cancel_stops_a_connect_promptly` |
| S16 | a lost session is `Unavailable` | `Lost` → `Other` | 6 tests (`…mid_read…`, `…mid_write…`, `a_lost_drive_is_reported_…`, `…media_gone…`, keepalive) |
| S17 | capabilities never change after connect | `atomic_rename` follows the connection | `after_the_connection_is_gone_every_call_is_unavailable_at_once` |
| S18 | keepalive closes a silent session | no keepalive | `keepalive_notices_a_silent_server_without_any_call` |
| S19 | the per-request timeout answers | request timeout 600 s (only the outer 3 s net left) | `a_stalled_server_times_out_instead_of_hanging` |
| S20 | no secret in errors | a rejected password is echoed in `ConnectError::Other` | `no_secret_shows_in_errors_or_debug_output`, `the_secret_is_exposed_only_where_…` (+2) |
| S21 | `create_dir` onto something is `AlreadyExists` | the look after `FAILURE` removed | conformance `create_dir_existing` |
| S22 | `rename` never overwrites | the `lstat(to)` check removed | `rename_refuses_an_existing_target_even_if_the_server_would_replace_it` (OpenSSH's rename refuses by itself) |
| S23 | no directory into its own subtree | check → `false` | differential (3 tests). Conformance alone does not catch it: rename(2) refuses with `EINVAL`, which maps to the same `Other` naming `to` |
| S24 | `open_read` past the end fails at open | size check disabled | conformance `read_offsets` |
| S25 | a half-written file is never under its final name | temporary name = final name | conformance `invisible_before_finish` |
| S26 | a directory is never written over | `is_dir` check removed | conformance `write_onto_directory` |
| S27 | `FAILURE` with disk-full text is `NoSpace` | text match disabled | `a_full_disk_is_no_space_and_leaves_nothing_behind` |
| S28 | «trust once» is not written down | append whatever `remember` says | `trust_once_is_kept_for_the_run_but_not_written` |
| S29 | a dead connection answers at once | `is_closed` short-circuit removed | **equivalent**: a request on a closed channel fails at once anyway («session closed») |
| S30 | the write directory is resolved once | `realpath` result ignored | differential, without posix-rename |
| S31 | `list` of a file is `Other` | the follow-up `stat` ignored | conformance `list_file_is_error` |
| S32 | with `key_file` only that agent identity is offered | filter disabled | `with_a_key_file_the_agent_only_offers_that_key` |

Not tested here: a real OpenSSH server (and its `realpath`, which may differ on
missing final components), latency and throughput over a real link, servers
speaking SFTP v4+ (status codes above 8, e.g. `FILE_ALREADY_EXISTS`, which
`russh-sftp` cannot decode), keyboard-interactive auth (not implemented), and
non-UTF-8 names (`russh-sftp` decodes them lossily).

## Object-store adapter (step 6)

All hermetic: `cargo test -p kara-remote` (the self dev-dependency enables `s3`
and `gcs`). Two doubles stand in for the services:

- `tests/objstore_support/`: `Faulty`, an `ObjectStore` + `PaginatedListStore`
  over `object_store::memory::InMemory` with call counts, a count of multipart
  uploads still open, paged listing (`page_size` keys per page), and faults per
  operation (head, get, body piece, put, start upload, part, complete, abort,
  list page, copy, delete): connection refused and 503 shaped like the real
  client's errors, 403, 401, 507 «storage full», `NoSuchBucket`, stalls, a body
  cut short, «unplugged» (every call refused), no conditional put, no
  copy-if-not-exists. Also the fake S3/GCS factories used with the registry.
- `tests/s3_mock_support/`: an S3 server on `127.0.0.1` (hyper) for the **real**
  AWS client of `object_store`. It checks the SigV4 signature of every request
  the way AWS documents it (path decoded and each segment re-encoded with the
  unreserved set, query sorted and encoded, signed headers, payload SHA-256),
  and serves ListObjectsV2 with continuation tokens, ranged `GET`, `HEAD`,
  `PUT` with `If-None-Match: *`, copy, bulk delete and multipart, with 503 and
  delay injection.

| File | Tests | What |
|---|---|---|
| `objstore_conformance.rs` | 6 | `conformance::run` + `run_extra`: over plain `InMemory`, paged (2 keys per page), without conditional put / copy-if-not-exists, under a prefix (nothing escapes it), with 4 KiB multipart parts; capabilities = `object_store_like()` and stable |
| `objstore_differential.rs` | 2 | seeded random ops vs `MemoryBackend::object_store_like()` (pages of 3, 16-byte parts), with and without conditional requests; 40 seeds × 100 ops each by default, **1000 seeds each passed once**; no divergence was ever found |
| `objstore_failures.rs` | 27 | failed part / completion / single put (no object, upload aborted), abort and drop abort, finish waits for every part, multipart and single-put races on `replace=false`, offsets, a lost and a truncated body, error kinds and paths without the key, unplugged service, hung request cut by the drive timeout, cancel between pages / during a page / in `remove_tree`, failed delete keeps placeholders, rename failing while copying / rolling back / deleting sources, no-overwrite and server-side `copy_within` (and a race on it), streamed copies above the copy limit, the placeholder hidden and reserved, empty-folder `remove`, object/folder name clash, names that are not keys |
| `objstore_factories.rs` | 14 | S3 and GCS parameters and defaults, the `allow_http` rule, secret-looking params refused, `~/` in the key file, prompts through the registry (missing → wrong → right, only the working secret kept; refused → `AuthRequired`; 3 wrong → `AuthFailed`), GCS never prompts, connect-time errors and cancel, the real AWS client against a closed port and a silent one (`Unreachable` in < 5 s / < 8 s), every call `Unavailable` on a dead endpoint, broken GCS key file reported without its content, nothing secret in `Debug` |
| `objstore_with_ops.rs` | 6 | `Lost` → reconnect with the data, upload/download of a folder (multipart, empty folders), copy and move inside the drive with **no `GET`**, cancel an upload (no object, no open upload), `MediaGone` mid-move keeps the source, confirmed permanent delete |
| `objstore_s3_client.rs` | 9 | the real client against the S3 server: conformance (pages of 2, under a prefix), 15 awkward names signed/listed/read/renamed, 12 MiB in 5 MiB parts and reads from offsets, abandoned upload aborted, `If-None-Match` race, server-side copy, paging over 50 entries + bulk delete, wrong secret / unknown key id → `AuthFailed` and the registry asks again, session token, 503 retried then `Unavailable` in < 6 s |
| `objstore_mutation_gaps.rs` | 4 | written after the mutation pass: cancel while a `remove_tree` page hangs, `AuthRequired` before any request, a part slower than the metadata bound, a 6 s download that keeps moving with `timeout_s=1` |
| `objstore_source_rules.rs` | 4 | no `unwrap`/`expect`/panicking macros, no printing in `src/objstore`; `Secret::expose` only in `s3.rs` (twice) |
| `objstore_real_service.rs` | 4 (ignored) | conformance against a real bucket (`KARA_TEST_S3`, `KARA_TEST_GCS`), `s3_throughput` (256 MiB, 1/4/8 parts in flight), and the same harness against the mock (loopback; checks the harness only) |

The S3 server found one bug before the mutation pass: `object_store` reports
a 401/403 on listing and bulk delete as `Generic` with the status only in the
text, so a wrong secret came out of `connect` as a plain error instead of
`AuthFailed`, and a denied listing as `Other`. Both are mapped now.

Not tested here: any real service (no network to a bucket, no MinIO, no GCS
emulator); the GCS client never made a request; throughput (only loopback
numbers against the mock exist, and they measure the mock); servers that ignore
`If-None-Match`; region redirects; keys `object_store` cannot parse; `object_store`'s
`LocalFileSystem` (not run: it has real directories, which this adapter does not
model).

### Mutation pass

Same method as above, over `crates/kara-remote/src/objstore/`: a script applies
one edit, runs `objstore_conformance`, `objstore_failures`,
`objstore_factories`, `objstore_with_ops`, `objstore_differential` and
`objstore_s3_client` (and, from the second round, `objstore_mutation_gaps`),
then `git checkout` + `touch`. 38 mutations: 35 caught at once, 2 survived and
are caught now, 1 equivalent.

| # | Mutation | File | Caught by |
|---|---|---|---|
| O01 | put_new: no check before the conditional put | `backend.rs` | `the_suite_passes_without_conditional_put_or_copy` |
| O02 | put_new: unconditional PUT after the check | `backend.rs` | `a_name_taken_between_the_check_and_the_put_is_not_overwritten` |
| O03 | multipart: no check before complete | `io.rs` | `the_loser_of_a_multipart_race_gets_already_exists` |
| O04 | abort_upload never aborts the multipart upload | `io.rs` | `the_suite_passes_with_small_multipart_parts`, `object_store_and_memory_model_agree`, `object_store_without_conditional_requests_and_memory_model_agree` |
| O05 | Drop does not abort | `io.rs` | `the_suite_passes_with_small_multipart_parts`, `object_store_without_conditional_requests_and_memory_model_agree`, `object_store_and_memory_model_agree` |
| O06 | a failed finish does not abort | `io.rs` | `a_failed_completion_leaves_no_object_and_aborts_the_upload`, `the_loser_of_a_multipart_race_gets_already_exists`, `a_connection_lost_mid_move_is_media_gone_and_keeps_the_source` |
| O07 | finish completes without waiting for the parts | `io.rs` | `big_files_go_up_in_parts_and_come_back_from_any_offset`, `a_connection_lost_mid_move_is_media_gone_and_keeps_the_source` |
| O08 | store errors name / instead of the caller's path | `error.rs` | `the_suite_passes_listing_page_by_page`, `the_suite_passes_over_in_memory`, `the_suite_passes_under_a_prefix_and_nothing_escapes_it` |
| O09 | the placeholder object is listed | `backend.rs` | `the_suite_passes_listing_page_by_page`, `the_suite_passes_over_in_memory`, `the_suite_passes_under_a_prefix_and_nothing_escapes_it` |
| O10 | rename deletes each source right after its copy | `backend.rs` | `a_rename_that_fails_while_copying_leaves_everything_under_the_old_name`, `sources_are_deleted_only_after_every_copy_and_a_late_failure_loses_nothing` |
| O11 | a failed rename does not take its copies back | `backend.rs` | `a_rename_that_fails_while_copying_leaves_everything_under_the_old_name` |
| O12 | remove_tree deletes placeholders together with the children | `backend.rs` | `a_failed_delete_in_remove_tree_keeps_the_placeholders_and_names_the_object`, `cancel_stops_remove_tree_midway_and_no_child_outlives_its_placeholder` |
| O13 | list ignores the token between and during later pages | `backend.rs` | `cancel_stops_a_listing_between_pages` |
| O14 | remove_tree ignores the token at each page (the in-batch check stays) | `backend.rs` | **survived** → `objstore_mutation_gaps::cancel_stops_remove_tree_while_a_page_is_on_its_way` |
| O15 | connection failures and 5xx are Other, not Unavailable | `error.rs` | `connect_time_errors_come_out_of_connect`, `a_dead_endpoint_is_unreachable_at_connect_promptly_and_says_nothing_secret`, `a_silent_endpoint_times_out_instead_of_hanging` |
| O16 | reads start at 0 whatever the offset | `io.rs` | `the_suite_passes_over_in_memory`, `the_suite_passes_under_a_prefix_and_nothing_escapes_it`, `the_suite_passes_listing_page_by_page` |
| O17 | server-side copies may overwrite | `backend.rs` | `a_copy_target_taken_during_the_copy_is_not_overwritten` |
| O18 | capabilities follow what the store turned out to support | `backend.rs` | `the_suite_passes_without_conditional_put_or_copy`, `the_suite_passes_through_the_real_client` |
| O19 | a missing secret reaches the service instead of AuthRequired | `s3.rs` | **survived** → `objstore_mutation_gaps::a_missing_s3_secret_is_auth_required_before_any_request` |
| O20 | plain http accepted without allow_http | `connect.rs` | `gcs_parameters_and_the_key_file_path`, `plain_http_needs_allow_http_and_allow_http_needs_plain_http` |
| O21 | objects may be created under an object | `backend.rs` | `object_store_and_memory_model_agree`, `object_store_without_conditional_requests_and_memory_model_agree` |
| O22 | a file may be written over a folder | `backend.rs` | `the_suite_passes_listing_page_by_page`, `the_suite_passes_under_a_prefix_and_nothing_escapes_it`, `the_suite_passes_over_in_memory` |
| O23 | a body that ends early is a short file | `io.rs` | `reads_start_at_the_offset_and_a_lost_body_is_unavailable` |
| O24 | S3 connect makes no request | `s3.rs` | `a_dead_endpoint_is_unreachable_at_connect_promptly_and_says_nothing_secret`, `a_refused_prompt_is_auth_required_and_three_wrong_secrets_are_auth_failed`, `a_missing_secret_is_asked_for_a_wrong_one_again_and_the_right_one_kept_if_asked` |
| O25 | a wrong S3 secret is not AuthFailed | `connect.rs` | **equivalent**: a 403 without `AccessDenied` is `AuthFailed` by the fallback rule anyway |
| O26 | remove of a non-empty folder removes its placeholder | `backend.rs` | `the_suite_passes_under_a_prefix_and_nothing_escapes_it`, `the_suite_passes_listing_page_by_page`, `the_suite_passes_over_in_memory` |
| O27 | objects above the copy limit are copied by the service anyway | `backend.rs` | `objects_above_the_server_copy_limit_are_streamed` |
| O28 | a folder may be renamed into itself | `backend.rs` | `the_suite_passes_over_in_memory`, `the_suite_passes_under_a_prefix_and_nothing_escapes_it`, `the_suite_passes_listing_page_by_page` |
| O29 | no bound on a hung call | `backend.rs` | `a_hung_request_is_cut_by_the_drive_timeout` |
| O30 | a missing folder lists as empty | `backend.rs` | `the_suite_passes_over_in_memory`, `the_suite_passes_listing_page_by_page`, `the_suite_passes_under_a_prefix_and_nothing_escapes_it` |
| O31 | listing a file lists the folder of the same name | `backend.rs` | `the_suite_passes_over_in_memory`, `the_suite_passes_under_a_prefix_and_nothing_escapes_it`, `the_suite_passes_listing_page_by_page` |
| O32 | 507 / storage full is not NoSpace | `error.rs` | `a_failed_single_put_leaves_no_object`, `errors_map_by_kind_and_name_the_callers_path_not_the_key` |
| O33 | the object key stays in the error text | `error.rs` | `errors_map_by_kind_and_name_the_callers_path_not_the_key` |
| O34 | the session token is not split off the secret | `s3.rs` | `a_session_token_after_the_secret_is_sent_and_signed` |
| O35 | AccessDenied with a valid key prompts for the secret again | `connect.rs` | `connect_time_errors_come_out_of_connect` |
| O36 | the whole request (body included) is bounded by 4 × `timeout_s` (the configuration before the fix below) | `connect.rs` | `a_long_download_that_keeps_moving_is_not_cut` |
| O37 | an upload part gets the metadata bound only | `backend.rs` | `a_slow_part_is_given_more_time_than_a_metadata_call` |
| O38 | an artificial leak: the secret appended to an `Unreachable` reason | `s3.rs` | `a_dead_endpoint_is_unreachable_at_connect_promptly_and_says_nothing_secret` (and the `expose()` count of `objstore_source_rules`) |

Why the survivors slipped through:

- **O14:** the check that follows each page still saw the token, only after
  the page arrived. The new test hangs the first page for 20 s and requires
  `Cancelled` within 2 s.
- **O19:** the fake service answers 401 without a secret, and the registry
  asks for the secret after `AuthFailed` as it does after `AuthRequired`, so
  the flow looked the same. The new test calls the factory directly and
  requires `AuthRequired` with no request made.

Found in review during the pass (not by a mutation): the clients were built
with `object_store`'s whole-request timeout (set to 4 × `timeout_s`), which
also bounds the response body, so a download longer than two minutes or an
upload part slower than about 70 KB/s was cut and retried until it failed.
Fixed (no whole-request timeout; read timeout `timeout_s`; parts bounded by
`MIN_UPLOAD_RATE`); O36 and O37 are its regression checks.
