/*!

This is a test-agent for running sonobuoy Kubernetes tests.
It expects to be run in a pod launched by the TestSys controller.

You can configure sonobuoy to run different types of plugins and tests.
See `SonobuoyConfig` for the different configuration values.

To build the container for the sonobuoy test agent, run `make sonobuoy-test-agent-image` from the
root directory of this repository.

Here is an example manifest for deploying the test definition for the sonobuoy test agent to a K8s cluster:

```yaml
apiVersion: testsys.bottlerocket.aws/v1
kind: Test
metadata:
  name: sonobuoy-e2e-full
  namespace: testsys-bottlerocket-aws
spec:
  agent:
    configuration:
      kubeconfig_base64: <Base64 encoded kubeconfig for the test cluster sonobuoy runs the tests in>
      plugin: e2e
      mode: certified-conformance
      kubernetes_version: v1.21.2
    image: <your sonobuoy-test-agent image URI>
    name: sonobuoy-test-agent
    keep_running: true
  resources: {}
```

!*/

use async_trait::async_trait;
use bottlerocket_agents::error::Error;
use bottlerocket_agents::sonobuoy::{delete_sonobuoy, rerun_failed_sonobuoy, run_sonobuoy};
use bottlerocket_agents::wireguard::setup_wireguard;
use bottlerocket_agents::{
    aws_test_config, base64_decode_write_file, error, init_agent_logger, E2E_REPO_CONFIG_PATH,
    TEST_CLUSTER_KUBECONFIG_PATH,
};
use bottlerocket_types::agent_config::{
    SonobuoyConfig, AWS_CREDENTIALS_SECRET_NAME, WIREGUARD_SECRET_NAME,
};
use log::{debug, info};
use model::{SecretName, TestResults};
use snafu::ResultExt;
use std::path::PathBuf;
use test_agent::{BootstrapData, ClientError, DefaultClient, Spec, TestAgent};

struct SonobuoyTestRunner {
    config: SonobuoyConfig,
    aws_secret_name: Option<SecretName>,
    wireguard_secret_name: Option<SecretName>,
    results_dir: PathBuf,
}

#[async_trait]
impl test_agent::Runner for SonobuoyTestRunner {
    type C = SonobuoyConfig;
    type E = Error;

    async fn new(spec: Spec<Self::C>) -> Result<Self, Self::E> {
        info!("Initializing Sonobuoy test agent...");
        Ok(Self {
            config: spec.configuration,
            aws_secret_name: spec.secrets.get(AWS_CREDENTIALS_SECRET_NAME).cloned(),
            wireguard_secret_name: spec.secrets.get(WIREGUARD_SECRET_NAME).cloned(),
            results_dir: spec.results_dir,
        })
    }

    async fn run(&mut self) -> Result<TestResults, Self::E> {
        aws_test_config(self, &self.aws_secret_name, &self.config.assume_role, &None).await?;

        if let Some(wireguard_secret_name) = &self.wireguard_secret_name {
            // If a wireguard secret is specified, try to set up an wireguard connection with the
            // wireguard configuration stored in the secret.
            let wireguard_secret = self
                .get_secret(wireguard_secret_name)
                .context(error::SecretMissingSnafu)?;
            setup_wireguard(&wireguard_secret).await?;
        }
        debug!("Decoding kubeconfig for test cluster");
        base64_decode_write_file(&self.config.kubeconfig_base64, TEST_CLUSTER_KUBECONFIG_PATH)
            .await?;
        info!("Stored kubeconfig in {}", TEST_CLUSTER_KUBECONFIG_PATH);
        let e2e_repo_config = match &self.config.e2e_repo_config_base64 {
            Some(e2e_repo_config_base64) => {
                info!("Decoding e2e-repo-config config");
                base64_decode_write_file(e2e_repo_config_base64, E2E_REPO_CONFIG_PATH).await?;
                info!("Stored e2e-repo-config in {}", E2E_REPO_CONFIG_PATH);
                Some(E2E_REPO_CONFIG_PATH)
            }
            None => None,
        };

        run_sonobuoy(
            TEST_CLUSTER_KUBECONFIG_PATH,
            e2e_repo_config,
            &self.config,
            &self.results_dir,
        )
        .await
    }

    async fn rerun_failed(&mut self, _prev_results: &TestResults) -> Result<TestResults, Self::E> {
        // Set up the aws credentials if they were provided.
        aws_test_config(self, &self.aws_secret_name, &self.config.assume_role, &None).await?;

        delete_sonobuoy(TEST_CLUSTER_KUBECONFIG_PATH).await?;

        debug!("Decoding kubeconfig for test cluster");
        base64_decode_write_file(&self.config.kubeconfig_base64, TEST_CLUSTER_KUBECONFIG_PATH)
            .await?;
        info!("Stored kubeconfig in {}", TEST_CLUSTER_KUBECONFIG_PATH);
        let e2e_repo_config = match &self.config.e2e_repo_config_base64 {
            Some(e2e_repo_config_base64) => {
                info!("Decoding e2e-repo-config config");
                base64_decode_write_file(e2e_repo_config_base64, E2E_REPO_CONFIG_PATH).await?;
                info!("Stored e2e-repo-config in {}", E2E_REPO_CONFIG_PATH);
                Some(E2E_REPO_CONFIG_PATH)
            }
            None => None,
        };

        rerun_failed_sonobuoy(
            TEST_CLUSTER_KUBECONFIG_PATH,
            e2e_repo_config,
            &self.config,
            &self.results_dir,
        )
        .await
    }

    async fn terminate(&mut self) -> Result<(), Self::E> {
        delete_sonobuoy(TEST_CLUSTER_KUBECONFIG_PATH).await
    }
}

#[tokio::main]
async fn main() {
    init_agent_logger(env!("CARGO_CRATE_NAME"), None);
    if let Err(e) = run().await {
        eprintln!("{}", e);
        std::process::exit(1);
    }
}

async fn run() -> Result<(), test_agent::error::Error<ClientError, Error>> {
    let mut agent = TestAgent::<DefaultClient, SonobuoyTestRunner>::new(
        BootstrapData::from_env().unwrap_or_else(|_| BootstrapData {
            test_name: "sonobuoy_test".to_string(),
        }),
    )
    .await?;
    agent.run().await
}
