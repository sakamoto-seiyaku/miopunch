//! Proposed Root-only offline signer fixture; no business/network endpoint.
//! Opens only an explicitly owned coherent snapshot prepared while provisioner
//! stopped. It is the same persisted administrator identity, not a new peer.
#[cfg(unix)]
mod enabled {
    use miopunch_native_probe::{
        daily_management::ManagementAction, management_auth::ManagementExtension,
        native_network::NativeNetwork,
    };
    use p2panda_auth::Access;
    use p2panda_core::{Hash, Header, VerifyingKey};
    use serde_json::{Value, json};
    use std::{error::Error, fs::OpenOptions, io::Write, path::PathBuf};

    type Failure = Box<dyn Error + Send + Sync>;

    fn private_file(path: &std::path::Path) -> std::io::Result<std::fs::File> {
        use std::os::unix::fs::OpenOptionsExt;
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
    }

    pub(super) async fn run() -> Result<Value, Failure> {
        let args = std::env::args().skip(1).collect::<Vec<_>>();
        if args.len() != 3 {
            return Err("usage: mio80_offline_removal_signer OWNED_SNAPSHOT TARGET_KEY NEW_OUTPUT_DIRECTORY".into());
        }
        let snapshot = std::fs::canonicalize(PathBuf::from(&args[0]))?;
        let target: VerifyingKey = args[1].parse()?;
        let requested_output = PathBuf::from(&args[2]);
        let output_parent = std::fs::canonicalize(
            requested_output
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .ok_or("output needs an existing owned parent")?,
        )?;
        let output = output_parent.join(requested_output.file_name().ok_or("output name missing")?);
        if output.exists() || output.starts_with(&snapshot) {
            return Err("output must be absent and outside owned snapshot".into());
        }
        // Root must prove the source provisioner is stopped and snapshot ownership
        // before launching this program. This program never opens the source store.
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new().mode(0o700).create(&output)?;
        }
        let mut node = NativeNetwork::open(&snapshot).await?;
        let authored = async {
            let before = node.status();
            if !before
                .members
                .iter()
                .any(|(id, access)| *id == before.node_id && *access == Access::manage())
            {
                return Err("snapshot node is not an actual current administrator".into());
            }
            if !before
                .members
                .iter()
                .any(|(id, access)| *id == target && *access == Access::read())
            {
                return Err("target is not an actual ordinary member in snapshot".into());
            }
            if target == before.node_id {
                return Err("fixture must remove ordinary Browser target, not signer".into());
            }
            let mut session = node.begin_management_session();
            let managed = node
                .manage(&mut session, ManagementAction::RemoveMember(target))
                .await;
            session.end();
            let managed = managed?;
            if managed.action != ManagementAction::RemoveMember(target)
                || managed.effective_access.is_some()
                || node.status().members.iter().any(|(id, _)| *id == target)
            {
                return Err("public removal did not apply locally".into());
            }
            let record = node
                .management_record(managed.record)
                .await?
                .ok_or("authored removal record absent")?;
            let decoded = Header::<ManagementExtension>::decode(record.header())?;
            if decoded.verifying_key != before.node_id
                || Hash::digest(record.header()) != managed.record
                || record.body().is_some()
            {
                return Err("actual author/hash/no-body record invariant failed".into());
            }
            let header = record.header().to_vec();
            Ok::<_, Failure>((managed.record, header, before.applied_count))
        }
        .await;
        node.close().await;
        let (id, header, before_count) = authored?;
        // Reopen only the signer snapshot to demonstrate its durable record and
        // effective view; the live original Native store is never touched.
        let reopened = NativeNetwork::open(&snapshot).await?;
        let durable = async {
            let record = reopened
                .management_record(id)
                .await?
                .ok_or("durable removal absent")?;
            if record.header() != header.as_slice()
                || record.body().is_some()
                || reopened
                    .status()
                    .members
                    .iter()
                    .any(|(member, _)| *member == target)
                || reopened.status().applied_count <= before_count
            {
                return Err("durable snapshot record/view mismatch".into());
            }
            Ok::<_, Failure>(())
        }
        .await;
        reopened.close().await;
        durable?;
        // Actual signed bytes are private controller input; never printed stdout.
        let mut envelope = private_file(&output.join("private-removal-record.json"))?;
        serde_json::to_writer(
            &mut envelope,
            &json!({"record":id.to_string(),"header":hex::encode(&header),"body":null}),
        )?;
        envelope.write_all(b"\n")?;
        envelope.sync_all()?;
        let receipt = json!({"event":"m80-offline-removal-authored",
        "record":id.to_string(),"headerHash":Hash::digest(&header).to_string(),
        "headerBytes":header.len(),"bodyPresent":false,"actualAdministrator":true,
        "ordinaryTargetBefore":true,"targetAbsentAfter":true,"durableRecordReopened":true,
        "publicManageUsed":true,"businessEndpointCreated":false,
        "samePersistedSnapshotIdentity":true,"independentParticipant":false});
        let mut saved = private_file(&output.join("signer-receipt.json"))?;
        serde_json::to_writer(&mut saved, &receipt)?;
        saved.write_all(b"\n")?;
        saved.sync_all()?;
        Ok(receipt)
    }
}

#[cfg(unix)]
#[tokio::main]
async fn main() {
    match enabled::run().await {
        Ok(receipt) => println!("{receipt}"),
        Err(error) => {
            // Raw CLI stderr/exit retained by Root; no private envelope dump.
            eprintln!("M80 offline signer failed: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(not(unix))]
fn main() {
    eprintln!(
        "M80 offline signer fixture unsupported on this platform; Unix private-file ownership required"
    );
    std::process::exit(1);
}
