use crate::errors::{
    AccountError, ChargebackError, DepositError, DisputeError, DuplicateTransactionError,
    ResolveError, StatementError, WithdrawalError,
};
use csv::Trim;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::{Entry, OccupiedEntry};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::path::PathBuf;

const PRECISION: f64 = 10_000.0;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TransactionType {
    /// A deposit is a credit to the client's asset account, meaning it should
    /// increase the available and total funds of the client account.
    Deposit,

    /// A withdraw is a debit to the client's asset account, meaning it should
    /// decrease the available and total funds of the client account.
    Withdrawal,

    /// A dispute represents a client's claim that a transaction was erroneous
    /// and should be reversed. The transaction shouldn't be reversed yet but
    /// the associated funds should be held. This means that the clients
    /// available funds should decrease by the amount disputed, their held
    /// funds should increase by the amount disputed, while their total funds
    /// should remain the same.
    Dispute,

    /// A resolve represents a resolution to a dispute, releasing the
    /// associated held funds. Funds that were previously disputed are no
    /// longer disputed. This means that the clients held funds should decrease
    /// by the amount no longer disputed, their available funds should increase
    /// by the amount no longer disputed, and their total funds should remain
    /// the same.
    Resolve,

    /// A chargeback is the final state of a dispute and represents the client
    /// reversing a transaction. Funds that were held have now been withdrawn.
    /// This means that the clients held funds and total funds should decrease
    /// by the amount previously disputed. If a chargeback occurs the client's
    /// account should be immediately frozen.
    Chargeback,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Transaction {
    r#type: TransactionType,
    client: u16,
    tx: u32,
    amount: Option<f64>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Account {
    client: u16,

    /// The total funds that are available for trading, staking, withdrawal,
    /// etc.
    available: f64,

    /// The total funds that are held for dispute.
    held: f64,

    /// The total funds that are available or held.
    total: f64,

    /// Whether the account is locked. An account is locked if a charge back
    /// occurs.
    locked: bool,
}

impl Account {
    pub fn new_account(client: u16, balance: f64) -> Self {
        let balance = Self::round(balance);
        Self {
            client,
            available: balance,
            held: 0.0,
            total: balance,
            locked: false,
        }
    }

    pub fn lock(&mut self) {
        self.locked = true;
    }

    pub fn deposit_funds(&mut self, amount: f64) {
        self.available = Self::round(self.available + amount);
        self.total = Self::round(self.calculate_total());
    }

    pub fn withdraw_funds(&mut self, amount: f64) {
        self.available = Self::round(self.available - amount);
        self.total = Self::round(self.calculate_total());
    }

    fn calculate_total(&self) -> f64 {
        self.available - self.held
    }

    fn round(value: f64) -> f64 {
        (value * PRECISION).round() / PRECISION
    }
}

/// A Ledger is responsible for processing a collection of Transactions and
/// tracking information about accounts, their balances, as well as any
/// disputes, resolutions, and chargebacks to those transactions.
#[derive(Default)]
pub struct Ledger {
    accounts: HashMap<u16, Account>,
    transactions: HashMap<u32, Transaction>,
    disputed_transactions: HashSet<u32>,
}

impl Ledger {
    /// Attempts to generate a CSV statement report for all accounts known to
    /// the Ledger.
    pub fn generate_account_statements(&self) -> Result<String, StatementError> {
        let mut buf = Vec::new();
        {
            // Create a CSV writer from the buffer we allocated above.
            let mut wtr = csv::Writer::from_writer(&mut buf);

            // Serialize each of the accounts to our output buffer.
            for account in self.accounts.values() {
                wtr.serialize(account)?;
            }

            // Flush the buffer.
            let _ = wtr.flush();
        }

        // Return the string contents of our buffer, bubbling up any UTF-8
        // encoding errors we encounter.
        Ok(String::from_utf8(buf)?)
    }

    /// Process a transaction of any supported type.
    pub fn process_transaction(&mut self, transaction: &Transaction) -> Result<(), Box<dyn Error>> {
        match transaction.r#type {
            TransactionType::Deposit => self.process_deposit(transaction)?,
            TransactionType::Withdrawal => {
                // We don't want to stop processing all of the data because a
                // single client attempted to overdraft their account, so log
                // an error message and keep moving if there were insufficient
                // funds. Any other errors should be bubbled up.
                if let Err(err) = self.process_withdrawal(transaction) {
                    if let WithdrawalError::InsufficientFunds(wanted, had) = err {
                        eprintln!(
                            "insufficient funds for transaction {} wanted={} had={}",
                            transaction.tx, wanted, had
                        );
                    } else {
                        return Err(Box::new(err));
                    }
                }
            }
            TransactionType::Dispute => self.process_dispute(transaction)?,
            TransactionType::Resolve => self.process_resolve(transaction)?,
            TransactionType::Chargeback => self.process_chargeback(transaction)?,
        };

        Ok(())
    }

    /// Process a deposit transaction.
    ///
    /// A deposit is a credit to a client's asset account, meaning it should
    /// increase the available and total funds of the client account.
    ///
    /// A positive amount MUST be specified in the provided transaction or an
    /// error will be returned. Locked accounts may NOT receive deposits.
    fn process_deposit(&mut self, transaction: &Transaction) -> Result<(), DepositError> {
        // Ensure that an amount was specified, otherwise return an error.
        let amount = match transaction.amount {
            None => return Err(DepositError::AmountRequired),
            Some(amount) => amount,
        };

        // If the specified amount was negative then return an error.
        if amount < 0.0 {
            return Err(DepositError::NegativeDeposit);
        }
        self.save_transaction(transaction)?;

        match self.accounts.entry(transaction.client) {
            Entry::Occupied(mut account) => {
                let account = account.get_mut();

                if account.locked {
                    return Err(DepositError::AccountLocked);
                }

                account.deposit_funds(amount);
            }
            Entry::Vacant(vacancy) => {
                vacancy.insert(Account::new_account(transaction.client, amount));
            }
        };

        Ok(())
    }

    /// Process a withdrawal transaction.
    ///
    /// A withdraw is a debit to the client's asset account, meaning it should
    /// decrease the available and total funds of the client account.
    ///
    /// A positive amount MUST be specified in the provided transaction or an
    /// error will be returned. Locked accounts may NOT withdraw funds.
    ///
    /// If a client does not have sufficient available funds the withdrawal
    /// will fail and the total amount of funds will not change.
    fn process_withdrawal(&mut self, transaction: &Transaction) -> Result<(), WithdrawalError> {
        // Ensure that an amount was specified, otherwise return an error.
        let amount = match transaction.amount {
            None => return Err(WithdrawalError::AmountRequired),
            Some(amount) => amount,
        };

        // If the specified amount was negative then return an error.
        if amount < 0.0 {
            return Err(WithdrawalError::NegativeWithdrawal);
        }
        self.save_transaction(transaction)?;

        match self.accounts.entry(transaction.client) {
            Entry::Occupied(mut account) => {
                let account = account.get_mut();

                if account.locked {
                    return Err(WithdrawalError::AccountLocked);
                }

                if account.available - amount < 0.0 {
                    return Err(WithdrawalError::InsufficientFunds(
                        amount,
                        account.available,
                    ));
                }

                account.withdraw_funds(amount);
            }
            Entry::Vacant(_) => return Err(WithdrawalError::NoSuchAccount(transaction.client)),
        };

        Ok(())
    }

    /// Process a dispute transaction.
    ///
    /// A dispute represents a client's claim that a transaction was erroneous
    /// and should be reversed. The transaction shouldn't be reversed yet but
    /// the associated funds should be held. This means that the clients
    /// available funds should decrease by the amount disputed, their held
    /// funds should increase by the amount disputed, while their total funds
    /// should remain the same.
    ///
    /// Note: a dispute does not state the amount disputed. Instead a dispute
    /// references the transaction that is disputed by ID. If the tx specified
    /// by the dispute doesn't exist it will be ignored and the assumption will
    /// be that this is an error on our partners side.
    fn process_dispute(&mut self, transaction: &Transaction) -> Result<(), DisputeError> {
        if let Entry::Occupied(mut tx) = self.transactions.entry(transaction.tx) {
            let tx = tx.get_mut();
            let amount = match tx.amount {
                Some(amount) => amount,
                None => return Err(DisputeError::AmountRequired),
            };

            let mut account = self.get_account_entry(transaction.client)?;
            let account = account.get_mut();

            account.available -= amount;
            account.held += amount;
            self.disputed_transactions.insert(transaction.tx);
        }

        Ok(())
    }

    /// Process a resolve transaction.
    ///
    /// A resolve represents a resolution to a dispute, releasing the
    /// associated held funds. Funds that were previously disputed are no
    /// longer disputed. This means that the clients held funds should decrease
    /// by the amount no longer disputed, their available funds should increase
    /// by the amount no longer disputed, and their total funds should remain
    /// the same.
    ///
    /// Note: Like disputes, resolves do not specify an amount. Instead they
    /// refer to a transaction that was under dispute by ID. If the tx
    /// specified doesn't exist, or the tx isn't under dispute, the resolve is
    /// ignored and the assumption is made that this is an error on our
    /// partner's side.
    fn process_resolve(&mut self, transaction: &Transaction) -> Result<(), ResolveError> {
        if let Entry::Occupied(tx) = self.transactions.entry(transaction.tx) {
            // If this transaction aims to resolve an undisputed transaction
            // then we simply skip over it.
            if !self.disputed_transactions.contains(&transaction.tx) {
                return Ok(());
            }

            let tx = tx.get();
            let amount = match tx.amount {
                Some(a) => a,
                None => return Err(ResolveError::AmountRequired),
            };

            let mut account = self.get_account_entry(transaction.client)?;
            let account = account.get_mut();

            account.available += amount;
            account.held -= amount;
            self.disputed_transactions.remove(&transaction.tx);
        }

        Ok(())
    }

    /// Process a chargeback transaction.
    ///
    /// A chargeback is the final state of a dispute and represents the client
    /// reversing a transaction. Funds that were held have now been withdrawn.
    /// This means that the clients held funds and total funds should decrease
    /// by the amount previously disputed. If a chargeback occurs the client's
    /// account should be immediately frozen.
    ///
    /// Note: Like a dispute and a resolve a chargeback refers to the
    /// transaction by ID (tx) and does not specify an amount. Like a resolve,
    /// if the tx specified doesn't exist, or the tx isn't under dispute, the
    /// chargeback will be ignored and the assumption will be made that this is
    /// an error on our partner's side.
    fn process_chargeback(&mut self, transaction: &Transaction) -> Result<(), ChargebackError> {
        if let Entry::Occupied(tx) = self.transactions.entry(transaction.tx) {
            // If this transaction aims to resolve an undisputed transaction
            // then we simply skip over it.
            if !self.disputed_transactions.contains(&transaction.tx) {
                return Ok(());
            }

            let tx = tx.get();
            let amount = match tx.amount {
                Some(a) => a,
                None => return Err(ChargebackError::AmountRequired),
            };

            let mut account = self.get_account_entry(transaction.client)?;
            let account = account.get_mut();
            account.held -= amount;
            account.total = account.available - account.held;
            account.lock();
        }

        Ok(())
    }

    /// Fetch attempt to fetch an OccupiedEntry which contains an existing
    /// Account.
    fn get_account_entry(&mut self, id: u16) -> Result<OccupiedEntry<u16, Account>, AccountError> {
        match self.accounts.entry(id) {
            Entry::Occupied(account) => {
                if account.get().locked {
                    return Err(AccountError::AccountLocked(id));
                }

                Ok(account)
            }
            Entry::Vacant(_) => Err(AccountError::NoSuchAccount(id)),
        }
    }

    fn save_transaction(
        &mut self,
        transaction: &Transaction,
    ) -> Result<(), DuplicateTransactionError> {
        match self.transactions.entry(transaction.tx) {
            Entry::Occupied(_) => return Err(DuplicateTransactionError::new(transaction.tx)),
            Entry::Vacant(entry) => {
                entry.insert(transaction.clone());
            }
        };

        Ok(())
    }
}

impl TryFrom<PathBuf> for Ledger {
    type Error = Box<dyn Error>;

    /// Attempts to parse the CSV file located at the provided PathBuf and
    /// streams the data into a newly allocated Ledger.
    ///
    /// Any errors encountered while decoding CSV rows or during transaction
    /// processing are returned immediately and the stream is closed.
    fn try_from(path: PathBuf) -> Result<Self, Self::Error> {
        // Allocate a new mutable ledger that we can populate from decoded CSV
        // transactions.
        let mut ledger = Ledger::default();

        // Create an iterator over all CSV records in the specified CSV file
        // path.
        //
        // Note: the csv library handles setting up an io::BufReader so we
        // don't need to do that here.
        let mut iter = csv::ReaderBuilder::new()
            .flexible(true)
            .has_headers(true)
            .trim(Trim::All)
            .from_path(path)?;

        for tx in iter.deserialize() {
            let tx: Transaction = tx?;
            ledger.process_transaction(&tx)?;
        }

        Ok(ledger)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Ledger {
        pub fn lock_account(&mut self, id: u16) {
            let mut account = self.get_account_entry(id).unwrap();
            let account = account.get_mut();

            account.lock();
        }

        pub fn process_transactions(
            &mut self,
            transactions: Vec<Transaction>,
        ) -> Result<(), Box<dyn Error>> {
            for tx in transactions.iter() {
                self.process_transaction(tx)?;
            }

            Ok(())
        }
    }

    #[test]
    fn should_fail_to_make_deposit_with_no_amount() {
        // Create a ledger and declare a client id to use.
        let mut ledger = Ledger::default();
        let client = 1u16;

        // Create and process a new deposit transaction with no amount set and
        // assert that the transaction fails with the expected error.
        let tx = Transaction {
            r#type: TransactionType::Deposit,
            client,
            tx: 1,
            amount: None,
        };
        assert_eq!(
            ledger.process_transaction(&tx).unwrap_err().to_string(),
            DepositError::AmountRequired.to_string()
        );
    }

    #[test]
    fn should_fail_to_deposit_negative_amount() {
        // Create a ledger and declare a client id to use.
        let mut ledger = Ledger::default();
        let client = 1u16;

        // Create and process a new negative deposit transaction and assert
        // that the transaction fails as expected.
        let tx = Transaction {
            r#type: TransactionType::Deposit,
            client,
            tx: 1,
            amount: Some(-1.0),
        };

        assert_eq!(
            ledger.process_transaction(&tx).unwrap_err().to_string(),
            DepositError::NegativeDeposit.to_string()
        );

        // Assert that we did not create a new account for the invalid deposit.
        assert_eq!(ledger.accounts.get(&client), None);
    }

    #[test]
    fn should_deposit_funds_to_new_account() {
        // Create a ledger and declare a client id to use.
        let mut ledger = Ledger::default();
        let client = 1u16;

        // Create and process a new deposit transaction and assert that the
        // transaction was processed successfully.
        let tx = Transaction {
            r#type: TransactionType::Deposit,
            client,
            tx: 1,
            amount: Some(1.0),
        };
        let result = ledger.process_transaction(&tx);
        assert!(result.is_ok());

        // Now assert that the account was created properly and that the
        // account contains the correct balance.
        assert_eq!(
            ledger.accounts.get(&client),
            Some(&Account {
                client,
                available: 1.0,
                held: 0.0,
                total: 1.0,
                locked: false
            })
        );
    }

    #[test]
    fn should_fail_to_deposit_duplicate_transaction() {
        // Create a ledger and declare a client id to use.
        let mut ledger = Ledger::default();
        let client = 1u16;

        // Create and publish the first transaction with id=1 and verify that
        // it processes successfully.
        let tx1 = Transaction {
            r#type: TransactionType::Deposit,
            client,
            tx: 1,
            amount: Some(1.0),
        };
        assert!(ledger.process_transaction(&tx1).is_ok());
        assert_eq!(
            ledger.accounts.get(&client),
            Some(&Account {
                client,
                available: 1.0,
                held: 0.0,
                total: 1.0,
                locked: false
            })
        );

        // Create and publish a second transaction with id=1 and verify that
        // the expected error is returned.
        let tx2 = Transaction {
            r#type: TransactionType::Deposit,
            client,
            tx: 1u32,
            amount: Some(1.0),
        };
        assert_eq!(
            ledger.process_transaction(&tx2).unwrap_err().to_string(),
            DepositError::DuplicateTx(DuplicateTransactionError::new(1)).to_string()
        );

        // Now verify that only the first deposit resulted in modifications to
        // the specified account.
        assert_eq!(
            ledger.accounts.get(&client),
            Some(&Account {
                client,
                available: 1.0,
                held: 0.0,
                total: 1.0,
                locked: false
            })
        );
    }

    #[test]
    fn should_deposit_multiple_transactions() {
        // Create a ledger and declare a client id to use.
        let mut ledger = Ledger::default();
        let client = 1u16;

        // Process two transactions and verify that they complete successfully.
        let tx1 = Transaction {
            r#type: TransactionType::Deposit,
            client,
            tx: 1,
            amount: Some(1.0),
        };
        let tx2 = Transaction {
            r#type: TransactionType::Deposit,
            client,
            tx: 2,
            amount: Some(1.0),
        };
        assert!(ledger.process_transactions(Vec::from([tx1, tx2])).is_ok());

        // Now verify the state of the account the deposits were made on.
        assert_eq!(
            ledger.accounts.get(&client),
            Some(&Account {
                client,
                available: 2.0,
                held: 0.0,
                total: 2.0,
                locked: false
            })
        );
    }

    #[test]
    fn should_fail_to_deposit_to_a_locked_account() {
        // Create a ledger and declare a client id to use.
        let mut ledger = Ledger::default();
        let client = 1u16;

        // Process a deposit and verify that it completes successfully.
        let tx1 = Transaction {
            r#type: TransactionType::Deposit,
            client,
            tx: 1,
            amount: Some(1.0),
        };
        assert!(ledger.process_transaction(&tx1).is_ok());

        // Now lock the account and attempt to make another deposit and verify
        // that the deposit fails with the expected error.
        ledger.lock_account(client);
        let tx2 = Transaction {
            r#type: TransactionType::Deposit,
            client,
            tx: 2,
            amount: Some(1.0),
        };
        assert_eq!(
            ledger.process_transaction(&tx2).unwrap_err().to_string(),
            DepositError::AccountLocked.to_string()
        );
    }

    #[test]
    fn should_fail_to_make_withdrawal_with_no_amount() {
        // Create a ledger and declare a client id to use.
        let mut ledger = Ledger::default();
        let client = 1u16;

        // Create and process a new withdrawal transaction with no amount set
        // and assert that the transaction fails with the expected error.
        let tx = Transaction {
            r#type: TransactionType::Withdrawal,
            client,
            tx: 1,
            amount: None,
        };
        assert_eq!(
            ledger.process_transaction(&tx).unwrap_err().to_string(),
            WithdrawalError::AmountRequired.to_string()
        );
    }

    #[test]
    fn should_fail_to_withdraw_negative_amount() {
        // Create a ledger and declare a client id to use.
        let mut ledger = Ledger::default();
        let client = 1u16;

        // Create and process a new negative withdrawal transaction and assert
        // that the transaction fails as expected.
        let tx = Transaction {
            r#type: TransactionType::Withdrawal,
            client,
            tx: 1,
            amount: Some(-1.0),
        };

        assert_eq!(
            ledger.process_transaction(&tx).unwrap_err().to_string(),
            WithdrawalError::NegativeWithdrawal.to_string()
        );
    }

    #[test]
    fn should_fail_to_withdraw_from_a_locked_account() {
        // Create a ledger and declare a client id to use.
        let mut ledger = Ledger::default();
        let client = 1u16;

        // Process a deposit and verify that it completes successfully.
        let tx1 = Transaction {
            r#type: TransactionType::Deposit,
            client,
            tx: 1,
            amount: Some(10.0),
        };
        assert!(ledger.process_transaction(&tx1).is_ok());

        // Now lock the account and attempt to make a withdrawal and verify
        // that the transaction fails with the expected error.
        ledger.lock_account(client);
        let tx2 = Transaction {
            r#type: TransactionType::Withdrawal,
            client,
            tx: 2,
            amount: Some(1.0),
        };
        assert_eq!(
            ledger.process_transaction(&tx2).unwrap_err().to_string(),
            WithdrawalError::AccountLocked.to_string()
        );
    }

    #[test]
    fn should_fail_to_withdraw_from_an_account_with_insufficient_funds() {
        // Create a ledger and declare a client id to use.
        let mut ledger = Ledger::default();
        let client = 1u16;

        // Process a deposit and verify that it completes successfully.
        let tx1 = Transaction {
            r#type: TransactionType::Deposit,
            client,
            tx: 1,
            amount: Some(10.0),
        };
        assert!(ledger.process_transaction(&tx1).is_ok());

        // Now attempt to withdraw more money than we just deposited and verify
        // we get the expected error.
        let tx2 = Transaction {
            r#type: TransactionType::Withdrawal,
            client,
            tx: 2,
            amount: Some(20.0),
        };
        assert_eq!(
            ledger.process_transaction(&tx2).unwrap_err().to_string(),
            WithdrawalError::InsufficientFunds(20.0, 10.0).to_string()
        );
    }

    #[test]
    fn should_fail_to_withdraw_from_an_unknown_account() {
        // Create a ledger and declare a client id to use.
        let mut ledger = Ledger::default();
        let client = 1u16;

        // Attempt to withdraw money from an unknown account.
        let tx1 = Transaction {
            r#type: TransactionType::Withdrawal,
            client,
            tx: 1,
            amount: Some(20.0),
        };
        assert_eq!(
            ledger.process_transaction(&tx1).unwrap_err().to_string(),
            WithdrawalError::NoSuchAccount(client).to_string()
        );
    }

    #[test]
    fn should_withdraw_from_account() {
        // Create a ledger and declare a client id to use.
        let mut ledger = Ledger::default();
        let client = 1u16;

        // Process a deposit and verify that it completes successfully.
        let tx1 = Transaction {
            r#type: TransactionType::Deposit,
            client,
            tx: 1,
            amount: Some(10.0),
        };
        assert!(ledger.process_transaction(&tx1).is_ok());

        for tx_id in 1..10 {
            let tx = Transaction {
                r#type: TransactionType::Withdrawal,
                client,
                tx: tx_id + 1, // Need to add 1 because we created tx 1 above
                amount: Some(1.0),
            };
            assert!(ledger.process_transaction(&tx).is_ok());
        }

        // Now verify the state of the account the withdrawals were made from.
        assert_eq!(
            ledger.accounts.get(&client),
            Some(&Account {
                client,
                available: 1.0,
                held: 0.0,
                total: 1.0,
                locked: false
            })
        );
    }

    #[test]
    fn should_fail_to_dispute_a_dispute() {
        // Create a ledger and declare a client id to use.
        let mut ledger = Ledger::default();
        let client = 1u16;

        // Process a deposit and verify that it completes successfully.
        let tx1 = Transaction {
            r#type: TransactionType::Deposit,
            client,
            tx: 1,
            amount: Some(1.0),
        };
        assert!(ledger.process_transaction(&tx1).is_ok());

        // Manually insert a dispute transaction.
        //
        // Note: We must manually insert this transaction since we _shouldn't_
        // otherwise be able to end up with an existing transaction that has no
        // amount.
        ledger.transactions.insert(
            2,
            Transaction {
                r#type: TransactionType::Dispute,
                client,
                tx: 1,
                amount: None,
            },
        );

        // Now attempt to dispute the dispute (tx with no amount) and verify
        // that the transaction fails.
        let tx3 = Transaction {
            r#type: TransactionType::Dispute,
            client,
            tx: 2,
            amount: None,
        };
        assert_eq!(
            ledger.process_transaction(&tx3).unwrap_err().to_string(),
            DisputeError::AmountRequired.to_string()
        );
    }

    #[test]
    fn should_dispute_a_deposit() {
        // Create a ledger and declare a client id to use.
        let mut ledger = Ledger::default();
        let client = 1u16;

        // Process a deposit and verify that it completes successfully.
        let tx1 = Transaction {
            r#type: TransactionType::Deposit,
            client,
            tx: 1,
            amount: Some(1.0),
        };
        assert!(ledger.process_transaction(&tx1).is_ok());

        // Now dispute that deposit and verify that we complete successfully
        // and that the ledger shows the transaction is disputed.
        let tx2 = Transaction {
            r#type: TransactionType::Dispute,
            client,
            tx: 1,
            amount: None,
        };
        assert!(ledger.process_transaction(&tx2).is_ok());
        assert!(ledger.disputed_transactions.contains(&1));
    }

    #[test]
    fn should_dispute_a_withdrawal() {
        // Create a ledger and declare a client id to use.
        let mut ledger = Ledger::default();
        let client = 1u16;

        // Process a couple transactions and verify that they complete
        // successfully.
        let tx1 = Transaction {
            r#type: TransactionType::Deposit,
            client,
            tx: 1,
            amount: Some(10.0),
        };
        let tx2 = Transaction {
            r#type: TransactionType::Withdrawal,
            client,
            tx: 2,
            amount: Some(5.0),
        };
        assert!(ledger.process_transactions(Vec::from([tx1, tx2])).is_ok());

        // Now dispute the withdrawal and verify that we complete successfully
        // and that the ledger shows the transaction is disputed.
        let tx3 = Transaction {
            r#type: TransactionType::Dispute,
            client,
            tx: 2,
            amount: None,
        };
        assert!(ledger.process_transaction(&tx3).is_ok());
        assert!(ledger.disputed_transactions.contains(&2));
    }

    #[test]
    fn should_resolve_a_disputed_transaction() {
        // Create a ledger and declare a client id to use.
        let mut ledger = Ledger::default();
        let client = 1u16;

        // Deposit funds into a new account, dispute the deposit, and then
        // resolve the dispute and assert the
        let tx1 = Transaction {
            r#type: TransactionType::Deposit,
            client,
            tx: 1,
            amount: Some(10.0),
        };
        let tx2 = Transaction {
            r#type: TransactionType::Dispute,
            client,
            tx: 1,
            amount: None,
        };
        let tx3 = Transaction {
            r#type: TransactionType::Resolve,
            client,
            tx: 1,
            amount: None,
        };
        assert!(ledger
            .process_transactions(Vec::from([tx1, tx2, tx3]))
            .is_ok());

        // Now assert that the account has the expected balance and that the
        // transaction is no longer disputed.
        assert_eq!(
            ledger.accounts.get(&client),
            Some(&Account {
                client,
                available: 10.0,
                held: 0.0,
                total: 10.0,
                locked: false
            })
        );
    }

    #[test]
    fn should_fail_to_resolve_a_transaction_with_no_amount() {
        // Create a ledger and declare a client id to use.
        let mut ledger = Ledger::default();
        let client = 1u16;

        // Process a deposit and verify that it completes successfully.
        let tx1 = Transaction {
            r#type: TransactionType::Deposit,
            client,
            tx: 1,
            amount: Some(1.0),
        };
        assert!(ledger.process_transaction(&tx1).is_ok());

        // Manually insert a dispute transaction and add it to the
        // disputed_transactions set.
        //
        // Note: We must manually insert this transaction since we _shouldn't_
        // otherwise be able to end up with an existing transaction that has no
        // amount.
        ledger.transactions.insert(
            2,
            Transaction {
                r#type: TransactionType::Dispute,
                client,
                tx: 1,
                amount: None,
            },
        );
        ledger.disputed_transactions.insert(2);

        // Now attempt to dispute the dispute (tx with no amount) and verify
        // that the transaction fails.
        let tx3 = Transaction {
            r#type: TransactionType::Resolve,
            client,
            tx: 2,
            amount: None,
        };
        assert_eq!(
            ledger.process_transaction(&tx3).unwrap_err().to_string(),
            ResolveError::AmountRequired.to_string()
        );
    }

    #[test]
    fn should_chargeback_a_disputed_transaction() {
        // Create a ledger and declare a client id to use.
        let mut ledger = Ledger::default();
        let client = 1u16;

        // Create two deposits, dispute the largest deposit, and then issue a
        // chargeback and verify that the result is successful.
        let tx1 = Transaction {
            r#type: TransactionType::Deposit,
            client,
            tx: 1,
            amount: Some(10.0),
        };
        let tx2 = Transaction {
            r#type: TransactionType::Deposit,
            client,
            tx: 2,
            amount: Some(1000.0),
        };
        let tx3 = Transaction {
            r#type: TransactionType::Dispute,
            client,
            tx: 2,
            amount: None,
        };
        let tx4 = Transaction {
            r#type: TransactionType::Chargeback,
            client,
            tx: 2,
            amount: None,
        };
        assert!(ledger
            .process_transactions(Vec::from([tx1, tx2, tx3, tx4]))
            .is_ok());

        // Now verify that the account shows the correct balance.
        assert_eq!(
            ledger.accounts.get(&client),
            Some(&Account {
                client,
                available: 10.0,
                held: 0.0,
                total: 10.0,
                locked: true
            })
        );
    }

    #[test]
    fn should_fail_to_chargeback_a_transaction_with_no_amount() {
        // Create a ledger and declare a client id to use.
        let mut ledger = Ledger::default();
        let client = 1u16;

        // Process a deposit and verify that it completes successfully.
        let tx1 = Transaction {
            r#type: TransactionType::Deposit,
            client,
            tx: 1,
            amount: Some(1.0),
        };
        assert!(ledger.process_transaction(&tx1).is_ok());

        // Manually insert a dispute transaction and add it to the
        // disputed_transactions set.
        //
        // Note: We must manually insert this transaction since we _shouldn't_
        // otherwise be able to end up with an existing transaction that has no
        // amount.
        ledger.transactions.insert(
            2,
            Transaction {
                r#type: TransactionType::Dispute,
                client,
                tx: 1,
                amount: None,
            },
        );
        ledger.disputed_transactions.insert(2);

        // Now attempt to chargeback the dispute (tx with no amount) and verify
        // that the transaction fails.
        let tx3 = Transaction {
            r#type: TransactionType::Chargeback,
            client,
            tx: 2,
            amount: None,
        };
        assert_eq!(
            ledger.process_transaction(&tx3).unwrap_err().to_string(),
            ChargebackError::AmountRequired.to_string()
        );
    }

    #[test]
    fn should_round_values() {
        // Create a ledger and declare a client id to use.
        let mut ledger = Ledger::default();
        let client = 1u16;

        // Deposit a value with more than 4 decimal points and verify that the
        // stored value is rounded to 4.
        let tx = Transaction {
            r#type: TransactionType::Deposit,
            client,
            tx: 1,
            amount: Some(8.675309),
        };
        assert!(ledger.process_transaction(&tx).is_ok());

        // Now verify that the account shows the correct balance.
        assert_eq!(
            ledger.accounts.get(&client),
            Some(&Account {
                client,
                available: 8.6753,
                held: 0.0,
                total: 8.6753,
                locked: false
            })
        );
    }

    #[test]
    fn should_generate_statement_report() {
        // Create a new ledger.
        let mut ledger = Ledger::default();

        // Deposit some values into the ledger.
        let tx1 = Transaction {
            r#type: TransactionType::Deposit,
            client: 1,
            tx: 1,
            amount: Some(10.0),
        };
        let tx2 = Transaction {
            r#type: TransactionType::Deposit,
            client: 2,
            tx: 2,
            amount: Some(20.0),
        };
        let tx3 = Transaction {
            r#type: TransactionType::Deposit,
            client: 3,
            tx: 3,
            amount: Some(30.0),
        };
        assert!(ledger
            .process_transactions(Vec::from([tx1, tx2, tx3]))
            .is_ok());

        // Now verify that the account shows the correct balance.
        let result = ledger.generate_account_statements();
        assert!(result.is_ok());

        // We don't guarantee a sort order for the output, so simply assert
        // that the lines we expect to see are present in the output.
        let output = result.unwrap();
        assert!(output.starts_with("client,available,held,total,locked\n"));
        let expected_lines = [
            "1,10.0,0.0,10.0,false\n",
            "2,20.0,0.0,20.0,false\n",
            "3,30.0,0.0,30.0,false\n",
        ];
        for line in expected_lines {
            assert!(output.contains(line));
        }
    }
}
