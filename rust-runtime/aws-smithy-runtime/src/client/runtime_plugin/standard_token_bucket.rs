/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use aws_smithy_runtime_api::client::runtime_plugin::RuntimePlugin;
use aws_smithy_types::config_bag::{FrozenLayer, Layer, Storable, StoreReplace};
use aws_smithy_types::retry::ErrorKind;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::trace;

/// A [RuntimePlugin] to provide a standard token bucket, usable by the
/// [`StandardRetryStrategy`](crate::client::retries::strategy::standard::StandardRetryStrategy).
#[non_exhaustive]
#[derive(Debug, Default)]
pub struct StandardTokenBucketRuntimePlugin {
    token_bucket: StandardTokenBucket,
}

impl StandardTokenBucketRuntimePlugin {
    pub fn new(initial_tokens: usize) -> Self {
        Self {
            token_bucket: StandardTokenBucket::new(initial_tokens),
        }
    }
}

impl RuntimePlugin for StandardTokenBucketRuntimePlugin {
    fn config(&self) -> Option<FrozenLayer> {
        let mut cfg = Layer::new("standard token bucket");
        cfg.store_put(self.token_bucket.clone());

        Some(cfg.freeze())
    }
}

const DEFAULT_CAPACITY: usize = 500;
const RETRY_COST: u32 = 5;
const RETRY_TIMEOUT_COST: u32 = RETRY_COST * 2;
const PERMIT_REGENERATION_AMOUNT: usize = 1;

#[derive(Clone, Debug)]
pub(crate) struct StandardTokenBucket {
    semaphore: Arc<Semaphore>,
    max_permits: usize,
    timeout_retry_cost: u32,
    retry_cost: u32,
}

impl Storable for StandardTokenBucket {
    type Storer = StoreReplace<Self>;
}

impl Default for StandardTokenBucket {
    fn default() -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(DEFAULT_CAPACITY)),
            max_permits: DEFAULT_CAPACITY,
            timeout_retry_cost: RETRY_TIMEOUT_COST,
            retry_cost: RETRY_COST,
        }
    }
}

impl StandardTokenBucket {
    pub(crate) fn new(initial_quota: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(initial_quota)),
            max_permits: initial_quota,
            retry_cost: RETRY_COST,
            timeout_retry_cost: RETRY_TIMEOUT_COST,
        }
    }

    pub(crate) fn acquire(&self, err: &ErrorKind) -> Option<OwnedSemaphorePermit> {
        let retry_cost = if err == &ErrorKind::TransientError {
            self.timeout_retry_cost
        } else {
            self.retry_cost
        };

        self.semaphore
            .clone()
            .try_acquire_many_owned(retry_cost)
            .ok()
    }

    pub(crate) fn regenerate_a_token(&self) {
        if self.semaphore.available_permits() < (self.max_permits) {
            trace!("adding {PERMIT_REGENERATION_AMOUNT} back into the bucket");
            self.semaphore.add_permits(PERMIT_REGENERATION_AMOUNT)
        }
    }

    #[cfg(all(test, feature = "test-util"))]
    pub(crate) fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }
}
