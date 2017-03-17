#[macro_use]
extern crate serde_derive;
extern crate blockchain_explorer;
extern crate sandbox;
extern crate exonum;
extern crate configuration_service;
extern crate serde_json;
extern crate serde;
#[macro_use]
extern crate log;

#[cfg(test)]
mod tests {

    use std::collections::BTreeMap;
    use exonum::crypto::{Hash, PublicKey, Seed, HASH_SIZE, gen_keypair_from_seed, hash};
    use exonum::storage::{Map, StorageValue, Error as StorageError};
    use exonum::messages::Message;
    use exonum::blockchain::config::StoredConfiguration;
    use exonum::blockchain::Service;
    use sandbox::sandbox_with_services;
    use sandbox::sandbox::Sandbox;
    use sandbox::timestamping::TimestampingService;
    use sandbox::sandbox_tests_helper::{SandboxState, add_one_height_with_transactions};
    use configuration_service::{TxConfigPropose, TxConfigVote, ConfigurationService,
                                ConfigurationSchema};
    use serde_json::Value;
    use serde_json::value::ToJson;
    use blockchain_explorer;

    pub fn configuration_sandbox() -> (Sandbox, SandboxState, StoredConfiguration) {
        let _ = blockchain_explorer::helpers::init_logger();
        let sandbox = sandbox_with_services(vec![Box::new(TimestampingService::new()),
                                                 Box::new(ConfigurationService::new())]);
        let sandbox_state = SandboxState::new();
        let initial_cfg = sandbox.cfg();
        (sandbox, sandbox_state, initial_cfg)
    }

    fn get_propose(sandbox: &Sandbox,
                   config_hash: Hash)
                   -> Result<Option<TxConfigPropose>, StorageError> {
        let view = sandbox.blockchain_ref().view();
        let schema = ConfigurationSchema::new(&view);
        let proposes_table = schema.config_proposes();
        proposes_table.get(&config_hash)
    }

    fn get_vote_for_propose(sandbox: &Sandbox,
                            config_hash: Hash,
                            pubkey: PublicKey)
                            -> Result<Option<TxConfigVote>, StorageError> {
        let view = sandbox.blockchain_ref().view();
        let schema = ConfigurationSchema::new(&view);
        let votes_table = schema.config_votes(config_hash);
        votes_table.get(&pubkey)
    }

    #[derive(Serialize)]
    struct CfgStub {
        cfg_string: String,
    }

    fn generate_config_with_message(actual_from: u64,
                                    timestamping_service_cfg_message: &str,
                                    sandbox: &Sandbox)
                                    -> StoredConfiguration {
        let mut services: BTreeMap<u16, Value> = BTreeMap::new();
        let tmstmp_id = TimestampingService::new().service_id();
        let service_cfg = CfgStub { cfg_string: timestamping_service_cfg_message.to_string() };
        services.insert(tmstmp_id, service_cfg.to_json());
        StoredConfiguration {
            actual_from: actual_from,
            validators: sandbox.validators(),
            consensus: sandbox.cfg().consensus,
            services: services,
        }
    }

    #[test]
    fn test_discard_proposes_with_expired_actual_from() {
        let (sandbox, sandbox_state, initial_cfg) = configuration_sandbox();

        let target_height = 10;
        for _ in 1..target_height {
            add_one_height_with_transactions(&sandbox, &sandbox_state, &[]);
        }
        sandbox.assert_state(target_height, 1);

        let new_cfg = generate_config_with_message(target_height, "First cfg", &sandbox);
        {
            let propose_tx = TxConfigPropose::new(&sandbox.p(1),
                                                  &initial_cfg.hash(),
                                                  &new_cfg.clone().serialize(),
                                                  sandbox.s(1));
            add_one_height_with_transactions(&sandbox, &sandbox_state, &[propose_tx.raw().clone()]);
            sandbox.assert_state(target_height + 1, 1);
            assert_eq!(None, get_propose(&sandbox, new_cfg.hash()).unwrap());
        }
    }

    #[test]
    fn test_discard_votes_with_expired_actual_from() {
        let (sandbox, sandbox_state, initial_cfg) = configuration_sandbox();
        sandbox.assert_state(1, 1);
        let target_height = 10;

        let new_cfg = generate_config_with_message(target_height, "First cfg", &sandbox);
        {
            let propose_tx = TxConfigPropose::new(&sandbox.p(1),
                                                  &initial_cfg.hash(),
                                                  &new_cfg.clone().serialize(),
                                                  sandbox.s(1));
            add_one_height_with_transactions(&sandbox, &sandbox_state, &[propose_tx.raw().clone()]);
            sandbox.assert_state(2, 1);
            assert_eq!(Some(propose_tx),
                       get_propose(&sandbox, new_cfg.hash()).unwrap());
        }
        {
            let legal_vote = TxConfigVote::new(&sandbox.p(3), &new_cfg.hash(), sandbox.s(3));
            add_one_height_with_transactions(&sandbox, &sandbox_state, &[legal_vote.raw().clone()]);
            sandbox.assert_state(3, 1);
            assert_eq!(Some(legal_vote),
                       get_vote_for_propose(&sandbox, new_cfg.hash(), sandbox.p(3)).unwrap());
        }
        {
            for _ in 3..target_height {
                add_one_height_with_transactions(&sandbox, &sandbox_state, &[]);
            }
            sandbox.assert_state(target_height, 1);
        }
        {
            let illegal_vote = TxConfigVote::new(&sandbox.p(0), &new_cfg.hash(), sandbox.s(0));
            add_one_height_with_transactions(&sandbox,
                                             &sandbox_state,
                                             &[illegal_vote.raw().clone()]);
            sandbox.assert_state(target_height + 1, 1);
            assert_eq!(None,
                       get_vote_for_propose(&sandbox, new_cfg.hash(), sandbox.p(0)).unwrap());
        }
    }

    #[test]
    fn test_discard_invalid_config_json() {
        let (sandbox, sandbox_state, initial_cfg) = configuration_sandbox();
        sandbox.assert_state(1, 1);
        let new_cfg = [70; 74]; //invalid json bytes
        let propose_tx =
            TxConfigPropose::new(&sandbox.p(1), &initial_cfg.hash(), &new_cfg, sandbox.s(1));
        add_one_height_with_transactions(&sandbox, &sandbox_state, &[propose_tx.raw().clone()]);
        sandbox.assert_state(2, 1);
        assert_eq!(None, get_propose(&sandbox, hash(&new_cfg)).unwrap());
    }

    #[test]
    fn test_change_service_config() {
        let (sandbox, sandbox_state, initial_cfg) = configuration_sandbox();

        let target_height = 10;
        for _ in 0..target_height {
            add_one_height_with_transactions(&sandbox, &sandbox_state, &[]);
        }
        sandbox.assert_state(target_height + 1, 1);

        let new_cfg = generate_config_with_message(target_height + 4, "First cfg", &sandbox);
        {
            let propose_tx = TxConfigPropose::new(&sandbox.p(1),
                                                  &initial_cfg.hash(),
                                                  &new_cfg.clone().serialize(),
                                                  sandbox.s(1));
            add_one_height_with_transactions(&sandbox, &sandbox_state, &[propose_tx.raw().clone()]);
            sandbox.assert_state(target_height + 2, 1);
            assert_eq!(propose_tx,
                       get_propose(&sandbox, new_cfg.hash()).unwrap().unwrap());
        }
        {
            let mut votes = Vec::new();
            for validator in 0..2 {
                votes.push(TxConfigVote::new(&sandbox.p(validator),
                                             &new_cfg.hash(),
                                             sandbox.s(validator))
                    .raw()
                    .clone());
            }
            add_one_height_with_transactions(&sandbox, &sandbox_state, &votes);
            sandbox.assert_state(target_height + 3, 1);
            for validator in 0..2 {
                assert_eq!(Some(&votes[validator]),
                           get_vote_for_propose(&sandbox, new_cfg.hash(), sandbox.p(validator))
                               .unwrap()
                               .map(|vote| vote.raw().clone())
                               .as_ref());
            }
            assert_eq!(None,
                       get_vote_for_propose(&sandbox, new_cfg.hash(), sandbox.p(2)).unwrap());
            assert_eq!(initial_cfg, sandbox.cfg());
            assert_eq!(None, sandbox.following_cfg());
        }
        {
            let vote3 = TxConfigVote::new(&sandbox.p(2), &new_cfg.hash(), sandbox.s(2));
            add_one_height_with_transactions(&sandbox, &sandbox_state, &[vote3.raw().clone()]);
            sandbox.assert_state(target_height + 4, 1);

            assert_eq!(Some(vote3),
                       get_vote_for_propose(&sandbox, new_cfg.hash(), sandbox.p(2)).unwrap());
            assert_eq!(new_cfg, sandbox.cfg());
        }
    }

    #[test]
    fn test_config_txs_discarded_when_following_config_present() {
        let (sandbox, sandbox_state, initial_cfg) = configuration_sandbox();
        sandbox.assert_state(1, 1);

        let following_config =
            generate_config_with_message(6, "Following cfg at height 6", &sandbox);

        {
            let propose_tx = TxConfigPropose::new(&sandbox.p(1),
                                                  &initial_cfg.hash(),
                                                  &following_config.clone().serialize(),
                                                  sandbox.s(1));
            add_one_height_with_transactions(&sandbox, &sandbox_state, &[propose_tx.raw().clone()]);
            sandbox.assert_state(2, 1);
            assert_eq!(Some(propose_tx),
                       get_propose(&sandbox, following_config.hash()).unwrap());
        }
        {
            let votes = (0..3)
                .map(|validator| {
                    TxConfigVote::new(&sandbox.p(validator),
                                      &following_config.hash(),
                                      sandbox.s(validator))
                        .raw()
                        .clone()
                })
                .collect::<Vec<_>>();
            add_one_height_with_transactions(&sandbox, &sandbox_state, &votes);
            sandbox.assert_state(3, 1);
            assert_eq!(sandbox.cfg(), initial_cfg);
            assert_eq!(sandbox.following_cfg(), Some(following_config.clone()));
        }
        let new_cfg = generate_config_with_message(7, "New cfg", &sandbox);

        {
            let propose_tx_new = TxConfigPropose::new(&sandbox.p(1),
                                                      &initial_cfg.hash(),
                                                      &new_cfg.clone().serialize(),
                                                      sandbox.s(1));
            add_one_height_with_transactions(&sandbox,
                                             &sandbox_state,
                                             &[propose_tx_new.raw().clone()]);
            sandbox.assert_state(4, 1);

            assert_eq!(None, get_propose(&sandbox, new_cfg.hash()).unwrap());
        }
        {
            let vote_validator_3 =
                TxConfigVote::new(&sandbox.p(3), &following_config.hash(), sandbox.s(3));
            add_one_height_with_transactions(&sandbox,
                                             &sandbox_state,
                                             &[vote_validator_3.raw().clone()]);
            sandbox.assert_state(5, 1);

            assert!(get_vote_for_propose(&sandbox, following_config.hash(), sandbox.p(0))
                .unwrap()
                .is_some());
            assert!(get_vote_for_propose(&sandbox, following_config.hash(), sandbox.p(3))
                .unwrap()
                .is_none());
            assert_eq!(initial_cfg, sandbox.cfg());
        }
        {
            add_one_height_with_transactions(&sandbox, &sandbox_state, &[]);
            sandbox.assert_state(6, 1);
            assert_eq!(following_config, sandbox.cfg());
        }
    }

    #[test]
    fn test_config_txs_discarded_when_not_referencing_actual_config_or_sent_by_illegal_validator
        () {
        let (sandbox, sandbox_state, initial_cfg) = configuration_sandbox();
        sandbox.assert_state(1, 1);

        let new_cfg = generate_config_with_message(6, "Following cfg at height 6", &sandbox);
        let new_cfg_discarded_votes = generate_config_with_message(8, "discarded votes", &sandbox);
        let (illegal_pub, illegal_sec) = gen_keypair_from_seed(&Seed::new([66; 32]));

        {
            let illegal_propose1 = TxConfigPropose::new(&sandbox.p(1),
                                                        &Hash::new([11; HASH_SIZE]), // not actual config hash
                                                        &new_cfg.clone().serialize(),
                                                        sandbox.s(1));
            let illegal_propose2 = TxConfigPropose::new(&illegal_pub, // not a member of actual config
                                                        &initial_cfg.hash(),
                                                        &new_cfg.clone().serialize(),
                                                        &illegal_sec);
            add_one_height_with_transactions(&sandbox,
                                             &sandbox_state,
                                             &[illegal_propose1.raw().clone(),
                                               illegal_propose2.raw().clone()]);
            sandbox.assert_state(2, 1);
            assert_eq!(None, get_propose(&sandbox, new_cfg.hash()).unwrap());
        }
        {
            let legal_propose1 = TxConfigPropose::new(&sandbox.p(1),
                                                      &initial_cfg.hash(),
                                                      &new_cfg.clone().serialize(),
                                                      sandbox.s(1));
            let legal_propose2 = TxConfigPropose::new(&sandbox.p(1),
                                                      &initial_cfg.hash(),
                                                      &new_cfg_discarded_votes.clone().serialize(),
                                                      sandbox.s(1));
            add_one_height_with_transactions(&sandbox,
                                             &sandbox_state,
                                             &[legal_propose1.raw().clone(),
                                               legal_propose2.raw().clone()]);
            sandbox.assert_state(3, 1);
            assert_eq!(Some(legal_propose1),
                       get_propose(&sandbox, new_cfg.hash()).unwrap());
            assert_eq!(Some(legal_propose2),
                       get_propose(&sandbox, new_cfg_discarded_votes.hash()).unwrap());
        }
        {
            let illegal_validator_vote =
                TxConfigVote::new(&illegal_pub, &new_cfg_discarded_votes.hash(), &illegal_sec);
            add_one_height_with_transactions(&sandbox,
                                             &sandbox_state,
                                             &[illegal_validator_vote.raw().clone()]);
            sandbox.assert_state(4, 1);
            assert_eq!(None,
                       get_vote_for_propose(&sandbox, new_cfg_discarded_votes.hash(), illegal_pub)
                           .unwrap());
        }
        {
            let votes = (0..3)
                .map(|validator| {
                    TxConfigVote::new(&sandbox.p(validator), &new_cfg.hash(), sandbox.s(validator))
                        .raw()
                        .clone()
                })
                .collect::<Vec<_>>();
            add_one_height_with_transactions(&sandbox, &sandbox_state, &votes);
            sandbox.assert_state(5, 1);
            assert_eq!(initial_cfg, sandbox.cfg());
            assert_eq!(Some(new_cfg.clone()), sandbox.following_cfg());
        }
        {
            add_one_height_with_transactions(&sandbox, &sandbox_state, &[]);
            sandbox.assert_state(6, 1);
            assert_eq!(new_cfg, sandbox.cfg());
            assert_eq!(None, sandbox.following_cfg());
        }
        {
            let votes = (0..3)
                .map(|validator| {
                    TxConfigVote::new(&sandbox.p(validator),
                                      &new_cfg_discarded_votes.hash(),
                                      sandbox.s(validator))
                        .raw()
                        .clone()
                })
                .collect::<Vec<_>>();
            add_one_height_with_transactions(&sandbox, &sandbox_state, &votes);
            sandbox.assert_state(7, 1);
            for validator in 0..3 {
                assert_eq!(None,
                           get_vote_for_propose(&sandbox,
                                                new_cfg_discarded_votes.hash(),
                                                sandbox.p(validator))
                               .unwrap());
            }
        }
    }

    /// regression: votes' were summed for all proposes simultaneously, and not for the same propose
    #[test]
    fn test_regression_majority_votes_for_different_proposes() {
        let (sandbox, sandbox_state, initial_cfg) = configuration_sandbox();
        sandbox.assert_state(1, 1);

        let actual_from = 5;

        let new_cfg1 = generate_config_with_message(actual_from, "First cfg", &sandbox);
        let new_cfg2 = generate_config_with_message(actual_from, "Second cfg", &sandbox);
        {
            let mut proposes = Vec::new();
            for cfg in &[new_cfg1.clone(), new_cfg2.clone()] {
                proposes.push(TxConfigPropose::new(&sandbox.p(1),
                                                   &initial_cfg.hash(),
                                                   &cfg.clone().serialize(),
                                                   sandbox.s(1))
                    .raw()
                    .clone());
            }

            add_one_height_with_transactions(&sandbox, &sandbox_state, &proposes);
            sandbox.assert_state(2, 1);
        }
        {
            let mut votes = Vec::new();
            for validator in 0..2 {
                votes.push(TxConfigVote::new(&sandbox.p(validator),
                                             &new_cfg1.hash(),
                                             sandbox.s(validator))
                    .raw()
                    .clone());
            }

            add_one_height_with_transactions(&sandbox, &sandbox_state, &votes);
            sandbox.assert_state(3, 1);
            assert_eq!(initial_cfg, sandbox.cfg());
        }
        {
            let prop2_validator2 = TxConfigVote::new(&sandbox.p(2), &new_cfg2.hash(), sandbox.s(2));

            add_one_height_with_transactions(&sandbox,
                                             &sandbox_state,
                                             &[prop2_validator2.raw().clone()]);
            sandbox.assert_state(4, 1);
            assert_eq!(initial_cfg, sandbox.cfg());
        }
        {
            let prop1_validator2 = TxConfigVote::new(&sandbox.p(2), &new_cfg1.hash(), sandbox.s(2));

            add_one_height_with_transactions(&sandbox,
                                             &sandbox_state,
                                             &[prop1_validator2.raw().clone()]);
            sandbox.assert_state(5, 1);
            assert_eq!(new_cfg1, sandbox.cfg());
        }
    }

    #[test]
    fn test_regression_new_vote_for_older_config_applies_old_config() {
        let (sandbox, sandbox_state, initial_cfg) = configuration_sandbox();
        sandbox.assert_state(1, 1);

        let new_cfg1 = generate_config_with_message(3, "First cfg", &sandbox);
        let new_cfg2 = generate_config_with_message(5, "Second cfg", &sandbox);

        {
            let propose_tx1 = TxConfigPropose::new(&sandbox.p(1),
                                                   &initial_cfg.hash(),
                                                   &new_cfg1.clone().serialize(),
                                                   sandbox.s(1));

            add_one_height_with_transactions(&sandbox,
                                             &sandbox_state,
                                             &[propose_tx1.raw().clone()]);
            sandbox.assert_state(2, 1);
        }
        {
            let mut votes_for_new_cfg1 = Vec::new();
            for validator in 0..3 {
                votes_for_new_cfg1.push(TxConfigVote::new(&sandbox.p(validator),
                                                          &new_cfg1.hash(),
                                                          sandbox.s(validator))
                    .raw()
                    .clone());
            }
            add_one_height_with_transactions(&sandbox, &sandbox_state, &votes_for_new_cfg1);
            sandbox.assert_state(3, 1);
            assert_eq!(new_cfg1, sandbox.cfg());
        }
        {
            let propose_tx2 = TxConfigPropose::new(&sandbox.p(1),
                                                   &new_cfg1.hash(),
                                                   &new_cfg2.clone().serialize(),
                                                   sandbox.s(1));

            add_one_height_with_transactions(&sandbox,
                                             &sandbox_state,
                                             &[propose_tx2.raw().clone()]);
            sandbox.assert_state(4, 1);
        }
        {
            let mut votes_for_new_cfg2 = Vec::new();
            for validator in 0..3 {
                votes_for_new_cfg2.push(TxConfigVote::new(&sandbox.p(validator),
                                                          &new_cfg2.hash(),
                                                          sandbox.s(validator))
                    .raw()
                    .clone());
            }
            add_one_height_with_transactions(&sandbox, &sandbox_state, &votes_for_new_cfg2);
            sandbox.assert_state(5, 1);
            assert_eq!(new_cfg2, sandbox.cfg());
        }
        let prop1_validator3 = TxConfigVote::new(&sandbox.p(3), &new_cfg1.hash(), sandbox.s(3));
        add_one_height_with_transactions(&sandbox,
                                         &sandbox_state,
                                         &[prop1_validator3.raw().clone()]);
        sandbox.assert_state(6, 1);
        assert_eq!(new_cfg2, sandbox.cfg());
    }
}