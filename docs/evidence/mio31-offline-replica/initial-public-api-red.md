# Initial public replica entry RED

Command actually run before the replica module/API existed:

```sh
rtk proxy cargo test --test offline_replica original_author_offline_restarted_member_supplies_body_and_dependencies -- --exact --nocapture
```

Actual exit: **101**. Compiler diagnostics included:

```text
error[E0432]: unresolved import `miopunch_native_probe::offline_replica`
error[E0599]: no method named `management_history` found for struct `NativeNetwork`
error[E0599]: no method named `sync_replica` found for struct `NativeNetwork`
error[E0599]: no method named `serve_replica` found for struct `NativeNetwork`
error[E0599]: no method named `management_record` found for struct `NativeNetwork`
```

This records the command, verdict and diagnostic excerpts; it is not presented
as a lossless capture of the original complete compiler output. The first real
successful scenario is retained separately in `first-real-check.log`.
