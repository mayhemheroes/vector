use std::sync::Arc;

use futures_util::{future, stream::BoxStream, FutureExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{oneshot, Mutex};
use vector_config::configurable_component;
use vector_core::config::LogNamespace;
use vector_core::{
    config::{DataType, Input, Output},
    event::Event,
    sink::{StreamSink, VectorSink},
};

use crate::{
    conditions::Condition,
    config::{AcknowledgementsConfig, SinkConfig, SinkContext, SourceConfig, SourceContext},
    impl_generate_config_from_default,
    sinks::Healthcheck,
    sources,
};

/// Configuration for the `unit_test` source.
#[configurable_component(source("unit_test"))]
#[derive(Clone, Debug, Default)]
pub struct UnitTestSourceConfig {
    /// List of events sent from this source as part of the test.
    #[serde(skip)]
    pub events: Vec<Event>,
}

impl_generate_config_from_default!(UnitTestSourceConfig);

#[async_trait::async_trait]
impl SourceConfig for UnitTestSourceConfig {
    async fn build(&self, cx: SourceContext) -> crate::Result<sources::Source> {
        let events = self.events.clone().into_iter();

        Ok(Box::pin(async move {
            let mut out = cx.out;
            let _shutdown = cx.shutdown;
            out.send_batch(events).await.map_err(|_| ())?;
            Ok(())
        }))
    }

    fn outputs(&self, _global_log_namespace: LogNamespace) -> Vec<Output> {
        vec![Output::default(DataType::all())]
    }

    fn can_acknowledge(&self) -> bool {
        false
    }
}

#[derive(Clone)]
pub enum UnitTestSinkCheck {
    /// Check all events that are received against the list of conditions.
    Checks(Vec<Vec<Condition>>),

    /// Check that no events were received.
    NoOutputs,

    /// Do nothing.
    NoOp,
}

impl Default for UnitTestSinkCheck {
    fn default() -> Self {
        UnitTestSinkCheck::NoOp
    }
}

#[derive(Debug)]
pub struct UnitTestSinkResult {
    pub test_name: String,
    pub test_errors: Vec<String>,
}

/// Configuration for the `unit_test` sink.
#[derive(Clone, Default, Derivative, Deserialize, Serialize)]
#[derivative(Debug)]
pub struct UnitTestSinkConfig {
    /// Name of the test that this sink is being used for.
    pub test_name: String,

    /// List of names of the transform/branch associated with this sink.
    pub transform_ids: Vec<String>,

    /// Sender side of the test result channel.
    #[serde(skip)]
    pub result_tx: Arc<Mutex<Option<oneshot::Sender<UnitTestSinkResult>>>>,

    /// Predicate applied to each event that reaches the sink.
    #[serde(skip)]
    #[derivative(Debug = "ignore")]
    pub check: UnitTestSinkCheck,
}

#[async_trait::async_trait]
#[typetag::serde(name = "unit_test")]
impl SinkConfig for UnitTestSinkConfig {
    async fn build(&self, _cx: SinkContext) -> crate::Result<(VectorSink, Healthcheck)> {
        let tx = self.result_tx.lock().await.take();
        let sink = UnitTestSink {
            test_name: self.test_name.clone(),
            transform_ids: self.transform_ids.clone(),
            result_tx: tx,
            check: self.check.clone(),
        };
        let healthcheck = future::ok(()).boxed();

        Ok((VectorSink::from_event_streamsink(sink), healthcheck))
    }

    fn sink_type(&self) -> &'static str {
        "unit_test"
    }

    fn input(&self) -> Input {
        Input::all()
    }

    fn acknowledgements(&self) -> &AcknowledgementsConfig {
        &AcknowledgementsConfig::DEFAULT
    }
}

pub struct UnitTestSink {
    pub test_name: String,
    pub transform_ids: Vec<String>,
    // None for NoOp test sinks
    pub result_tx: Option<oneshot::Sender<UnitTestSinkResult>>,
    pub check: UnitTestSinkCheck,
}

#[async_trait::async_trait]
impl StreamSink<Event> for UnitTestSink {
    async fn run(mut self: Box<Self>, mut input: BoxStream<'_, Event>) -> Result<(), ()> {
        let mut output_events = Vec::new();
        let mut result = UnitTestSinkResult {
            test_name: self.test_name,
            test_errors: Vec::new(),
        };

        while let Some(event) = input.next().await {
            output_events.push(event);
        }

        match self.check {
            UnitTestSinkCheck::Checks(checks) => {
                if output_events.is_empty() {
                    result
                        .test_errors
                        .push(format!("checks for transforms {:?} failed: no events received. Topology may be disconnected or transform is missing inputs.", self.transform_ids));
                } else {
                    for (i, check) in checks.iter().enumerate() {
                        let mut check_errors = Vec::new();
                        for (j, condition) in check.iter().enumerate() {
                            let mut condition_errors = Vec::new();
                            for event in output_events.iter() {
                                match condition.check_with_context(event.clone()).0 {
                                    Ok(_) => {
                                        condition_errors.clear();
                                        break;
                                    }
                                    Err(error) => {
                                        condition_errors
                                            .push(format!("  condition[{}]: {}", j, error));
                                    }
                                }
                            }
                            check_errors.extend(condition_errors);
                        }
                        // If there are errors, add a preamble to the output
                        if !check_errors.is_empty() {
                            check_errors.insert(
                                0,
                                format!(
                                    "check[{}] for transforms {:?} failed conditions:",
                                    i, self.transform_ids
                                ),
                            );
                        }

                        result.test_errors.extend(check_errors);
                    }

                    // If there are errors, add a summary of events received
                    if !result.test_errors.is_empty() {
                        result.test_errors.push(format!(
                            "output payloads from {:?} (events encoded as JSON):\n  {}",
                            self.transform_ids,
                            events_to_string(&output_events)
                        ));
                    }
                }
            }
            UnitTestSinkCheck::NoOutputs => {
                if !output_events.is_empty() {
                    result.test_errors.push(format!(
                        "check for transforms {:?} failed: expected no outputs",
                        self.transform_ids
                    ));
                }
            }
            UnitTestSinkCheck::NoOp => {}
        }

        if let Some(tx) = self.result_tx {
            if tx.send(result).is_err() {
                error!(message = "Sending unit test results failed in unit test sink.");
            }
        }
        Ok(())
    }
}

fn events_to_string(events: &[Event]) -> String {
    events
        .iter()
        .map(|event| match event {
            Event::Log(log) => serde_json::to_string(log).unwrap_or_else(|_| "{}".to_string()),
            Event::Metric(metric) => {
                serde_json::to_string(metric).unwrap_or_else(|_| "{}".to_string())
            }
            Event::Trace(trace) => {
                serde_json::to_string(trace).unwrap_or_else(|_| "{}".to_string())
            }
        })
        .collect::<Vec<_>>()
        .join("\n  ")
}
