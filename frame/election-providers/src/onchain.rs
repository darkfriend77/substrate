// This file is part of Substrate.

// Copyright (C) 2020 Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// 	http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! An implementation of [`sp_election_providers::ElectionProvider`] that does an on-chain
//! sequential phragmen.

use sp_arithmetic::PerThing;
use sp_election_providers::{ElectionDataProvider, ElectionProvider};
use sp_npos_elections::{
	ElectionResult, ExtendedBalance, IdentifierT, PerThing128, Supports, VoteWeight,
};
use sp_runtime::RuntimeDebug;
use sp_std::{collections::btree_map::BTreeMap, marker::PhantomData, prelude::*};

/// Errors of the on-chain election.
#[derive(RuntimeDebug, Eq, PartialEq)]
pub enum Error {
	/// An internal error in the NPoS elections crate.
	NposElections(sp_npos_elections::Error),
	/// An assignment failed to pass the feasibility check
	Feasibility(&'static str),
}

impl From<sp_npos_elections::Error> for Error {
	fn from(e: sp_npos_elections::Error) -> Self {
		Error::NposElections(e)
	}
}

/// A simple on-chian implementation of the election provider trait.
///
/// This will accept voting data on the fly and produce the results immediately.
///
/// ### Warning
///
/// This can be very expensive to run frequently on-chain. Use with care.
pub struct OnChainSequentialPhragmen<T: Config>(PhantomData<T>);

/// Configuration trait of [`OnChainSequentialPhragmen`].
///
/// Note that this is similar to a pallet traits, but [`OnChainSequentialPhragmen`] is not a pallet.
pub trait Config {
	/// The account identifier type.
	type AccountId: IdentifierT;
	/// The block number type.
	type BlockNumber;
	/// Something that provides the data for election.
	type ElectionDataProvider: ElectionDataProvider<Self::AccountId, Self::BlockNumber>;
}

impl<T: Config> ElectionProvider<T::AccountId> for OnChainSequentialPhragmen<T> {
	type Error = Error;

	fn elect<P: PerThing128>() -> Result<Supports<T::AccountId>, Self::Error>
	where
		ExtendedBalance: From<<P as PerThing>::Inner>,
	{
		let voters = T::ElectionDataProvider::voters();
		let targets = T::ElectionDataProvider::targets();
		let desired_targets = T::ElectionDataProvider::desired_targets() as usize;

		let mut stake_map: BTreeMap<T::AccountId, VoteWeight> = BTreeMap::new();

		voters.iter().for_each(|(v, s, _)| {
			stake_map.insert(v.clone(), *s);
		});

		let stake_of = Box::new(|w: &T::AccountId| -> VoteWeight {
			stake_map.get(w).cloned().unwrap_or_default()
		});

		let ElectionResult {
			winners,
			assignments,
		} = sp_npos_elections::seq_phragmen::<_, P>(desired_targets, targets, voters, None)
			.map_err(Error::from)?;

		// check all assignments for feasibility, based on election data provider.
		assignments
			.iter()
			.map(T::ElectionDataProvider::feasibility_check_assignment)
			.collect::<Result<_, _>>()
			.map_err(|e| Error::Feasibility(e))?;

		let staked =
			sp_npos_elections::assignment_ratio_to_staked_normalized(assignments, &stake_of)?;
		let winners = sp_npos_elections::to_without_backing(winners);

		sp_npos_elections::to_supports(&winners, &staked).map_err(Error::from)
	}

	fn ongoing() -> bool {
		false
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use sp_election_providers::{Assignment, VoteWeight};
	use sp_npos_elections::Support;

	type AccountId = u64;
	type BlockNumber = u32;
	type Accuracy = sp_runtime::Perbill;

	struct Runtime;
	impl Config for Runtime {
		type AccountId = AccountId;
		type BlockNumber = BlockNumber;
		type ElectionDataProvider = mock_data_provider::DataProvider;
	}

	type OnChainPhragmen = OnChainSequentialPhragmen<Runtime>;

	mod mock_data_provider {
		use super::*;

		pub struct DataProvider;

		impl ElectionDataProvider<AccountId, BlockNumber> for DataProvider {
			fn voters() -> Vec<(AccountId, VoteWeight, Vec<AccountId>)> {
				vec![
					(1, 10, vec![10, 20]),
					(2, 20, vec![30, 20]),
					(3, 30, vec![10, 30]),
				]
			}

			fn targets() -> Vec<AccountId> {
				vec![10, 20, 30]
			}

			fn desired_targets() -> u32 {
				2
			}

			fn feasibility_check_assignment<P: PerThing>(
				_: &Assignment<AccountId, P>,
			) -> Result<(), &'static str> {
				Ok(())
			}

			fn next_election_prediction(_: BlockNumber) -> BlockNumber {
				0
			}
		}
	}

	#[test]
	fn onchain_seq_phragmen_works() {
		assert_eq!(
			OnChainPhragmen::elect::<Accuracy>().unwrap(),
			vec![
				(
					10,
					Support {
						total: 25,
						voters: vec![(1, 10), (3, 15)]
					}
				),
				(
					30,
					Support {
						total: 35,
						voters: vec![(2, 20), (3, 15)]
					}
				)
			]
		);
	}
}
