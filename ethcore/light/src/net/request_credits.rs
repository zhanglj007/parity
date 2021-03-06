// Copyright 2015-2017 Parity Technologies (UK) Ltd.
// This file is part of Parity.

// Parity is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Parity is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Parity.  If not, see <http://www.gnu.org/licenses/>.

//! Request credit management.
//!
//! Every request in the light protocol leads to a reduction
//! of the requester's amount of credits as a rate-limiting mechanism.
//! The amount of credits will recharge at a set rate.
//!
//! This module provides an interface for configuration of
//! costs and recharge rates of request credits.
//!
//! Current default costs are picked completely arbitrarily, not based
//! on any empirical timings or mathematical models.

use request::{self, Request};
use super::error::Error;

use rlp::*;
use ethereum_types::U256;
use time::{Duration, SteadyTime};

/// Credits value.
///
/// Produced and recharged using `FlowParams`.
/// Definitive updates can be made as well -- these will reset the recharge
/// point to the time of the update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credits {
	estimate: U256,
	recharge_point: SteadyTime,
}

impl Credits {
	/// Get the current amount of credits..
	pub fn current(&self) -> U256 { self.estimate.clone() }

	/// Make a definitive update.
	/// This will be the value obtained after receiving
	/// a response to a request.
	pub fn update_to(&mut self, value: U256) {
		self.estimate = value;
		self.recharge_point = SteadyTime::now();
	}

	/// Maintain ratio to current limit against an old limit.
	pub fn maintain_ratio(&mut self, old_limit: U256, new_limit: U256) {
		self.estimate = (new_limit * self.estimate) / old_limit;
	}

	/// Attempt to apply the given cost to the amount of credits.
	///
	/// If successful, the cost will be deducted successfully.
	///
	/// If unsuccessful, the structure will be unaltered an an
	/// error will be produced.
	pub fn deduct_cost(&mut self, cost: U256) -> Result<(), Error> {
		match cost > self.estimate {
			true => Err(Error::NoCredits),
			false => {
				self.estimate = self.estimate - cost;
				Ok(())
			}
		}
	}
}

/// A cost table, mapping requests to base and per-request costs.
/// Costs themselves may be missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CostTable {
	base: U256, // cost per packet.
	headers: Option<U256>, // cost per header
	transaction_index: Option<U256>,
	body: Option<U256>,
	receipts: Option<U256>,
	account: Option<U256>,
	storage: Option<U256>,
	code: Option<U256>,
	header_proof: Option<U256>,
	transaction_proof: Option<U256>, // cost per gas.
	epoch_signal: Option<U256>,
}

impl CostTable {
	fn costs_set(&self) -> usize {
		let mut num_set = 0;

		{
			let mut incr_if_set = |cost: &Option<_>| if cost.is_some() { num_set += 1 };
			incr_if_set(&self.headers);
			incr_if_set(&self.transaction_index);
			incr_if_set(&self.body);
			incr_if_set(&self.receipts);
			incr_if_set(&self.account);
			incr_if_set(&self.storage);
			incr_if_set(&self.code);
			incr_if_set(&self.header_proof);
			incr_if_set(&self.transaction_proof);
			incr_if_set(&self.epoch_signal);
		}

		num_set
	}
}

impl Default for CostTable {
	fn default() -> Self {
		// arbitrarily chosen constants.
		CostTable {
			base: 100000.into(),
			headers: Some(10000.into()),
			transaction_index: Some(10000.into()),
			body: Some(15000.into()),
			receipts: Some(5000.into()),
			account: Some(25000.into()),
			storage: Some(25000.into()),
			code: Some(20000.into()),
			header_proof: Some(15000.into()),
			transaction_proof: Some(2.into()),
			epoch_signal: Some(10000.into()),
		}
	}
}

impl Encodable for CostTable {
	fn rlp_append(&self, s: &mut RlpStream) {
		fn append_cost(s: &mut RlpStream, cost: &Option<U256>, kind: request::Kind) {
			if let Some(ref cost) = *cost {
				s.begin_list(2);
				// hack around https://github.com/paritytech/parity/issues/4356
				Encodable::rlp_append(&kind, s);
				s.append(cost);
			}
		}

		s.begin_list(1 + self.costs_set()).append(&self.base);
		append_cost(s, &self.headers, request::Kind::Headers);
		append_cost(s, &self.transaction_index, request::Kind::TransactionIndex);
		append_cost(s, &self.body, request::Kind::Body);
		append_cost(s, &self.receipts, request::Kind::Receipts);
		append_cost(s, &self.account, request::Kind::Account);
		append_cost(s, &self.storage, request::Kind::Storage);
		append_cost(s, &self.code, request::Kind::Code);
		append_cost(s, &self.header_proof, request::Kind::HeaderProof);
		append_cost(s, &self.transaction_proof, request::Kind::Execution);
		append_cost(s, &self.epoch_signal, request::Kind::Signal);
	}
}

impl Decodable for CostTable {
	fn decode(rlp: &UntrustedRlp) -> Result<Self, DecoderError> {
		let base = rlp.val_at(0)?;

		let mut headers = None;
		let mut transaction_index = None;
		let mut body = None;
		let mut receipts = None;
		let mut account = None;
		let mut storage = None;
		let mut code = None;
		let mut header_proof = None;
		let mut transaction_proof = None;
		let mut epoch_signal = None;

		for cost_list in rlp.iter().skip(1) {
			let cost = cost_list.val_at(1)?;
			match cost_list.val_at(0)? {
				request::Kind::Headers => headers = Some(cost),
				request::Kind::TransactionIndex => transaction_index = Some(cost),
				request::Kind::Body => body = Some(cost),
				request::Kind::Receipts => receipts = Some(cost),
				request::Kind::Account => account = Some(cost),
				request::Kind::Storage => storage = Some(cost),
				request::Kind::Code => code = Some(cost),
				request::Kind::HeaderProof => header_proof = Some(cost),
				request::Kind::Execution => transaction_proof = Some(cost),
				request::Kind::Signal => epoch_signal = Some(cost),
			}
		}

		let table = CostTable {
			base: base,
			headers: headers,
			transaction_index: transaction_index,
			body: body,
			receipts: receipts,
			account: account,
			storage: storage,
			code: code,
			header_proof: header_proof,
			transaction_proof: transaction_proof,
			epoch_signal: epoch_signal,
		};

		if table.costs_set() == 0 {
			Err(DecoderError::Custom("no cost types set."))
		} else {
			Ok(table)
		}
	}
}

/// Handles costs, recharge, limits of request credits.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowParams {
	costs: CostTable,
	limit: U256,
	recharge: U256,
}

impl FlowParams {
	/// Create new flow parameters from a request cost table,
	/// credit limit, and (minimum) rate of recharge.
	pub fn new(limit: U256, costs: CostTable, recharge: U256) -> Self {
		FlowParams {
			costs: costs,
			limit: limit,
			recharge: recharge,
		}
	}

	/// Create new flow parameters from ,
	/// proportion of total capacity which should be given to a peer,
	/// and number of seconds of stored capacity a peer can accumulate.
	pub fn from_request_times<F: Fn(::request::Kind) -> u64>(
		request_time_ns: F,
		load_share: f64,
		max_stored_seconds: u64
	) -> Self {
		use request::Kind;

		let load_share = load_share.abs();

		let recharge: u64 = 100_000_000;
		let max = recharge.saturating_mul(max_stored_seconds);

		let cost_for_kind = |kind| {
			// how many requests we can handle per second
			let ns = request_time_ns(kind);
			let second_duration = 1_000_000_000f64 / ns as f64;

			// scale by share of the load given to this peer.
			let serve_per_second = second_duration * load_share;
			let serve_per_second = serve_per_second.max(1.0 / 10_000.0);

			// as a percentage of the recharge per second.
			Some(U256::from((recharge as f64 / serve_per_second) as u64))
		};

		let costs = CostTable {
			base: 0.into(),
			headers: cost_for_kind(Kind::Headers),
			transaction_index: cost_for_kind(Kind::TransactionIndex),
			body: cost_for_kind(Kind::Body),
			receipts: cost_for_kind(Kind::Receipts),
			account: cost_for_kind(Kind::Account),
			storage: cost_for_kind(Kind::Storage),
			code: cost_for_kind(Kind::Code),
			header_proof: cost_for_kind(Kind::HeaderProof),
			transaction_proof: cost_for_kind(Kind::Execution),
			epoch_signal: cost_for_kind(Kind::Signal),
		};

		FlowParams {
			costs: costs,
			limit: max.into(),
			recharge: recharge.into(),
		}
	}

	/// Create effectively infinite flow params.
	pub fn free() -> Self {
		let free_cost: Option<U256> = Some(0.into());
		FlowParams {
			limit: (!0u64).into(),
			recharge: 1.into(),
			costs: CostTable {
				base: 0.into(),
				headers: free_cost.clone(),
				transaction_index: free_cost.clone(),
				body: free_cost.clone(),
				receipts: free_cost.clone(),
				account: free_cost.clone(),
				storage: free_cost.clone(),
				code: free_cost.clone(),
				header_proof: free_cost.clone(),
				transaction_proof: free_cost.clone(),
				epoch_signal: free_cost,
			}
		}
	}

	/// Get a reference to the credit limit.
	pub fn limit(&self) -> &U256 { &self.limit }

	/// Get a reference to the cost table.
	pub fn cost_table(&self) -> &CostTable { &self.costs }

	/// Get the base cost of a request.
	pub fn base_cost(&self) -> U256 { self.costs.base }

	/// Get a reference to the recharge rate.
	pub fn recharge_rate(&self) -> &U256 { &self.recharge }

	/// Compute the actual cost of a request, given the kind of request
	/// and number of requests made.
	pub fn compute_cost(&self, request: &Request) -> Option<U256> {
		match *request {
			Request::Headers(ref req) => self.costs.headers.map(|c| c * req.max.into()),
			Request::HeaderProof(_) => self.costs.header_proof,
			Request::TransactionIndex(_) => self.costs.transaction_index,
			Request::Body(_) => self.costs.body,
			Request::Receipts(_) => self.costs.receipts,
			Request::Account(_) => self.costs.account,
			Request::Storage(_) => self.costs.storage,
			Request::Code(_) => self.costs.code,
			Request::Execution(ref req) => self.costs.transaction_proof.map(|c| c * req.gas),
			Request::Signal(_) => self.costs.epoch_signal,
		}
	}

	/// Compute the cost of a set of requests.
	/// This is the base cost plus the cost of each individual request.
	pub fn compute_cost_multi(&self, requests: &[Request]) -> Option<U256> {
		let mut cost = self.costs.base;
		for request in requests {
			match self.compute_cost(request) {
				Some(c) => cost = cost + c,
				None => return None,
			}
		}

		Some(cost)
	}

	/// Create initial credits.
	pub fn create_credits(&self) -> Credits {
		Credits {
			estimate: self.limit,
			recharge_point: SteadyTime::now(),
		}
	}

	/// Recharge the given credits based on time passed since last
	/// update.
	pub fn recharge(&self, credits: &mut Credits) {
		let now = SteadyTime::now();

		// recompute and update only in terms of full seconds elapsed
		// in order to keep the estimate as an underestimate.
		let elapsed = (now - credits.recharge_point).num_seconds();
		credits.recharge_point = credits.recharge_point + Duration::seconds(elapsed);

		let elapsed: U256 = elapsed.into();

		credits.estimate = ::std::cmp::min(self.limit, credits.estimate + (elapsed * self.recharge));
	}

	/// Refund some credits which were previously deducted.
	/// Does not update the recharge timestamp.
	pub fn refund(&self, credits: &mut Credits, refund_amount: U256) {
		credits.estimate = credits.estimate + refund_amount;

		if credits.estimate > self.limit {
			credits.estimate = self.limit
		}
	}
}

impl Default for FlowParams {
	fn default() -> Self {
		FlowParams {
			limit: 50_000_000.into(),
			costs: CostTable::default(),
			recharge: 100_000.into(),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn should_serialize_cost_table() {
		let costs = CostTable::default();
		let serialized = ::rlp::encode(&costs);

		let new_costs: CostTable = ::rlp::decode(&*serialized);

		assert_eq!(costs, new_costs);
	}

	#[test]
	fn credits_mechanism() {
		use std::thread;
		use std::time::Duration;

		let flow_params = FlowParams::new(100.into(), Default::default(), 20.into());
		let mut credits =  flow_params.create_credits();

		assert!(credits.deduct_cost(101.into()).is_err());
		assert!(credits.deduct_cost(10.into()).is_ok());

		thread::sleep(Duration::from_secs(1));

		flow_params.recharge(&mut credits);

		assert_eq!(credits.estimate, 100.into());
	}

	#[test]
	fn scale_by_load_share_and_time() {
		let flow_params = FlowParams::from_request_times(
			|_| 10_000,
			0.05,
			60,
		);

		let flow_params2 = FlowParams::from_request_times(
			|_| 10_000,
			0.1,
			60,
		);

		let flow_params3 = FlowParams::from_request_times(
			|_| 5_000,
			0.05,
			60,
		);

		assert_eq!(flow_params2.costs, flow_params3.costs);
		assert_eq!(flow_params.costs.headers.unwrap(), flow_params2.costs.headers.unwrap() * 2.into());
	}
}
