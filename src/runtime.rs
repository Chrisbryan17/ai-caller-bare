use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{
    ActiveSetManager, ActiveSetProposal, CandidateEvaluation, CandidateSignal, CopybotError,
    DynamicWatcherSet, ExecutionFill, LaunchController, LiveOutcome, MarketResolution,
    PaperOutcome, Result, RotationContext, Trade, WalletLifecycle, WalletRegistry, WatcherSpec,
};

const RUNTIME_SCHEMA_VERSION: u32 = 1;
pub const MAX_WATCHED_WALLETS: usize = 4;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PaperPosition {
    pub wallet: String,
    pub market_end_epoch: i64,
    pub counts_global: bool,
    #[serde(default)]
    pub live: bool,
    pub fill: ExecutionFill,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PaperResolutionRecord {
    pub wallet: String,
    pub condition_id: String,
    pub resolved_epoch: i64,
    pub won: bool,
    pub pnl: Decimal,
    pub counts_global: bool,
    pub live: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedRuntimeState {
    schema_version: u32,
    launch_controller: LaunchController,
    positions: Vec<PaperPosition>,
}

pub struct RotationRuntime {
    registry: WalletRegistry,
    watchers: DynamicWatcherSet,
    active_set_manager: ActiveSetManager,
    launch_controller: LaunchController,
    positions: Vec<PaperPosition>,
    runtime_state_path: PathBuf,
}

impl RotationRuntime {
    pub fn open(
        registry_path: impl AsRef<Path>,
        bankroll: Decimal,
        started_epoch: i64,
    ) -> Result<Self> {
        let registry_path = registry_path.as_ref().to_path_buf();
        let runtime_state_path = registry_path.with_extension("runtime.json");
        let registry = WalletRegistry::load_or_new(&registry_path, bankroll)?;
        let persisted = load_runtime_state(&runtime_state_path)?;
        let (launch_controller, positions) = persisted.map_or_else(
            || (LaunchController::new(started_epoch, bankroll), Vec::new()),
            |state| (state.launch_controller, state.positions),
        );
        let mut runtime = Self {
            registry,
            watchers: DynamicWatcherSet::new(10_000)?,
            active_set_manager: ActiveSetManager::default(),
            launch_controller,
            positions,
            runtime_state_path,
        };
        runtime.synchronize_watchers()?;
        Ok(runtime)
    }

    #[must_use]
    pub fn registry(&self) -> &WalletRegistry {
        &self.registry
    }

    pub fn registry_mut(&mut self) -> &mut WalletRegistry {
        &mut self.registry
    }

    #[must_use]
    pub fn launch_controller(&self) -> &LaunchController {
        &self.launch_controller
    }

    pub fn launch_controller_mut(&mut self) -> &mut LaunchController {
        &mut self.launch_controller
    }

    #[must_use]
    pub fn watched_wallets(&self) -> Vec<String> {
        self.watchers.wallets()
    }

    #[must_use]
    pub fn pending_positions(&self) -> &[PaperPosition] {
        &self.positions
    }

    pub fn apply_evaluations(
        &mut self,
        evaluations: Vec<CandidateEvaluation>,
        now: i64,
    ) -> Result<()> {
        for evaluation in evaluations {
            self.registry.upsert_evaluation(evaluation, now);
        }
        self.registry.state_mut().last_successful_discovery_epoch = Some(now);
        self.synchronize_watchers()?;
        self.registry.save_atomic()?;
        self.save_runtime_state()?;
        Ok(())
    }

    pub fn process_snapshot(
        &mut self,
        wallet: &str,
        trades: Vec<Trade>,
        now: i64,
    ) -> Result<Vec<CandidateSignal>> {
        let was_primed = self.watchers.is_primed(wallet);
        match self.watchers.process_snapshot(wallet, trades) {
            Ok(signals) => {
                if !was_primed && self.watchers.is_primed(wallet) {
                    self.registry.mark_primed(wallet, now)?;
                    self.registry.save_atomic()?;
                }
                Ok(signals)
            }
            Err(error) => {
                let _ = self.registry.record_processing_error(wallet);
                let _ = self.registry.save_atomic();
                Err(error)
            }
        }
    }

    pub fn rotate(&mut self, context: RotationContext, live: bool) -> Result<ActiveSetProposal> {
        let current = self.registry.state().active_wallets.clone();
        let proposal = self
            .active_set_manager
            .propose(&self.registry, &current, context);
        if proposal.deferred || !proposal.changed {
            return Ok(proposal);
        }
        let mut proposed_registry = self.registry.clone();
        proposed_registry.apply_active_wallets(proposal.wallets.clone(), live);
        proposed_registry.save_atomic()?;
        self.registry = proposed_registry;
        self.synchronize_watchers()?;
        self.save_runtime_state()?;
        Ok(proposal)
    }

    pub fn track_paper_fill(
        &mut self,
        wallet: &str,
        market_end_epoch: i64,
        counts_global: bool,
        fill: ExecutionFill,
    ) -> Result<()> {
        let wallet = wallet.to_ascii_lowercase();
        if self.positions.iter().any(|position| {
            position.wallet == wallet && position.fill.condition_id == fill.condition_id
        }) {
            return Ok(());
        }
        self.positions.push(PaperPosition {
            wallet,
            market_end_epoch,
            counts_global,
            live: false,
            fill,
        });
        self.positions.sort_by(|left, right| {
            (left.market_end_epoch, &left.wallet, &left.fill.condition_id).cmp(&(
                right.market_end_epoch,
                &right.wallet,
                &right.fill.condition_id,
            ))
        });
        self.save_runtime_state()
    }

    pub fn track_live_fill(
        &mut self,
        wallet: &str,
        market_end_epoch: i64,
        fill: ExecutionFill,
    ) -> Result<()> {
        if fill.paper {
            return Err(CopybotError::InvalidConfiguration(
                "live position cannot contain a paper fill".into(),
            ));
        }
        let wallet = wallet.to_ascii_lowercase();
        if self.positions.iter().any(|position| {
            position.wallet == wallet && position.fill.condition_id == fill.condition_id
        }) {
            return Ok(());
        }
        self.positions.push(PaperPosition {
            wallet,
            market_end_epoch,
            counts_global: false,
            live: true,
            fill,
        });
        self.positions.sort_by(|left, right| {
            (left.market_end_epoch, &left.wallet, &left.fill.condition_id).cmp(&(
                right.market_end_epoch,
                &right.wallet,
                &right.fill.condition_id,
            ))
        });
        self.save_runtime_state()
    }

    pub fn due_condition_ids(&self, now: i64) -> Vec<String> {
        let mut condition_ids: Vec<String> = self
            .positions
            .iter()
            .filter(|position| position.market_end_epoch <= now)
            .map(|position| position.fill.condition_id.clone())
            .collect();
        condition_ids.sort();
        condition_ids.dedup();
        condition_ids
    }

    pub fn resolve_positions(
        &mut self,
        resolutions: &HashMap<String, MarketResolution>,
        now: i64,
    ) -> Result<Vec<PaperResolutionRecord>> {
        let mut remaining = Vec::new();
        let mut resolved = Vec::new();
        for position in self.positions.drain(..) {
            let Some(resolution) = resolutions.get(&position.fill.condition_id) else {
                remaining.push(position);
                continue;
            };
            let Some(winner) = resolution.winner else {
                remaining.push(position);
                continue;
            };
            if !resolution.closed {
                remaining.push(position);
                continue;
            }
            let won = position.fill.outcome == winner;
            let pnl = if won {
                position.fill.shares - position.fill.total_cost
            } else {
                -position.fill.total_cost
            };
            if position.live {
                self.registry.record_live_outcome(
                    &position.wallet,
                    LiveOutcome {
                        condition_id: position.fill.condition_id.clone(),
                        resolved_epoch: now,
                        pnl,
                        won,
                    },
                )?;
            } else {
                self.registry.record_paper_outcome(
                    &position.wallet,
                    PaperOutcome {
                        condition_id: position.fill.condition_id.clone(),
                        resolved_epoch: now,
                        pnl,
                        won,
                    },
                )?;
                self.registry.refresh_qualification(&position.wallet, now)?;
                if position.counts_global {
                    self.launch_controller.record_resolved(pnl, won);
                }
            }
            resolved.push(PaperResolutionRecord {
                wallet: position.wallet,
                condition_id: position.fill.condition_id,
                resolved_epoch: now,
                won,
                pnl,
                counts_global: position.counts_global,
                live: position.live,
            });
        }
        self.positions = remaining;
        self.synchronize_watchers()?;
        self.registry.save_atomic()?;
        self.save_runtime_state()?;
        Ok(resolved)
    }

    pub fn resolve_paper(
        &mut self,
        resolutions: &HashMap<String, MarketResolution>,
        now: i64,
    ) -> Result<Vec<PaperResolutionRecord>> {
        self.resolve_positions(resolutions, now)
    }

    fn synchronize_watchers(&mut self) -> Result<()> {
        let active: HashSet<String> = self
            .registry
            .state()
            .active_wallets
            .iter()
            .map(|wallet| wallet.to_ascii_lowercase())
            .collect();
        let mut records: Vec<_> = self
            .registry
            .state()
            .records
            .values()
            .filter(|record| {
                matches!(
                    record.lifecycle,
                    WalletLifecycle::Quarantined
                        | WalletLifecycle::PaperQualified
                        | WalletLifecycle::ActivePaper
                        | WalletLifecycle::ActiveLive
                )
            })
            .filter_map(|record| {
                record.family.map(|family| {
                    (
                        active.contains(&record.wallet),
                        record.score(),
                        WatcherSpec {
                            wallet: record.wallet.clone(),
                            family,
                            estimated_win_probability: record.estimated_win_probability,
                        },
                    )
                })
            })
            .collect();
        records.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| right.1.cmp(&left.1))
                .then_with(|| left.2.wallet.cmp(&right.2.wallet))
        });
        let specs: Vec<WatcherSpec> = records
            .into_iter()
            .take(MAX_WATCHED_WALLETS)
            .map(|(_, _, spec)| spec)
            .collect();
        self.watchers.synchronize(&specs)
    }

    fn save_runtime_state(&self) -> Result<()> {
        if let Some(parent) = self.runtime_state_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = self.runtime_state_path.with_extension("runtime.tmp");
        let state = PersistedRuntimeState {
            schema_version: RUNTIME_SCHEMA_VERSION,
            launch_controller: self.launch_controller.clone(),
            positions: self.positions.clone(),
        };
        let encoded = serde_json::to_vec_pretty(&state)?;
        let mut file = File::create(&temporary)?;
        file.write_all(&encoded)?;
        file.sync_all()?;
        fs::rename(temporary, &self.runtime_state_path)?;
        Ok(())
    }
}

fn load_runtime_state(path: &Path) -> Result<Option<PersistedRuntimeState>> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path)?;
    let state: PersistedRuntimeState = serde_json::from_slice(&bytes)?;
    if state.schema_version != RUNTIME_SCHEMA_VERSION {
        return Err(CopybotError::InvalidConfiguration(format!(
            "unsupported runtime schema {}",
            state.schema_version
        )));
    }
    Ok(Some(state))
}
