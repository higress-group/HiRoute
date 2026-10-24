//! Restart the same isolated listener with its current publication and durable roots.
use super::*;
use hiroute_gateway::server::publication::GatewayPublicationSnapshotV3;

impl RuntimeFixture {
    pub fn restart_with_publication(&mut self, snapshot: &GatewayPublicationSnapshotV3) {
        snapshot.validate().unwrap();
        let mut child = self.process.child.take().expect("running fixture");
        child.kill().unwrap();
        child.wait().unwrap();
        let directory = self._directory.path();
        let publication = directory.join("publication.json");
        std::fs::write(&publication, serde_json::to_vec_pretty(snapshot).unwrap()).unwrap();
        let nonce_file = directory.join("runtime-state-control.nonce");
        if let Some((_, nonce)) = &self.runtime_state_control {
            let mut open = std::fs::OpenOptions::new();
            open.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                open.mode(0o600);
            }
            let mut file = open.open(&nonce_file).unwrap();
            file.write_all(nonce.as_bytes()).unwrap();
            file.sync_all().unwrap();
        }
        self.process = Hirouted::spawn(HiroutedSpawn {
            binary: &exact_hirouted_binary(),
            address: self.address,
            lkg: &directory.join("publication-lkg.json"),
            publication: &publication,
            credentials: &directory.join("credentials.json"),
            directory,
            replay_root: &self.replay_root,
            observation_root: self.observation_root.as_deref(),
            test_control: self
                .runtime_state_control
                .as_ref()
                .map(|(address, _)| (*address, nonce_file.as_path())),
            options: self.launch_options,
        });
        self.process.wait_ready();
        assert!(!nonce_file.exists());
    }
}
