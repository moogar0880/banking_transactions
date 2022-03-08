use crate::errors::{
    AccountError, ChargebackError, DepositError, DisputeError, DuplicateTransactionError,
    ResolveError, WithdrawalError,
};
use csv::Trim;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::{Entry, OccupiedEntry};
use std::collections::{HashMap, HashSet};
use std::error::Error;

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

#[derive(Debug, Serialize)]
pub struct Account {
    client: u16,
    available: f64,
    held: f64,
    total: f64,
    locked: bool,
}

impl Account {
    pub fn new_account(client: u16, balance: f64) -> Self {
        Self {
            client,
            available: balance,
            held: 0.0,
            total: balance,
            locked: false,
        }
    }
}

pub fn read_all(path: &str) -> Result<Vec<Transaction>, Box<dyn Error>> {
    let mut iter = csv::ReaderBuilder::new()
        .flexible(true)
        .has_headers(true)
        .trim(Trim::All)
        .from_path(path)?;

    let mut output = Vec::new();
    for txn in iter.deserialize() {
        output.push(txn?);
    }

    Ok(output)
}

pub struct Ledger {
    accounts: HashMap<u16, Account>,
    transactions: HashMap<u32, Transaction>,
    disputed_transactions: HashSet<u32>,
}

impl Ledger {
    pub fn new() -> Self {
        Self {
            accounts: HashMap::new(),
            transactions: HashMap::new(),
            disputed_transactions: HashSet::new(),
        }
    }

    pub fn process_transactions(
        &mut self,
        transactions: Vec<Transaction>,
    ) -> Result<(), Box<dyn Error>> {
        for tx in transactions.iter() {
            if let Err(err) = self.process_transaction(tx) {
                return Err(err);
            }
        }

        Ok(())
    }

    pub fn process_transaction(&mut self, transaction: &Transaction) -> Result<(), Box<dyn Error>> {
        match transaction.r#type {
            TransactionType::Deposit => self.process_deposit(transaction)?,
            TransactionType::Withdrawal => self.process_withdrawal(transaction)?,
            TransactionType::Dispute => self.process_dispute(transaction)?,
            TransactionType::Resolve => self.process_resolve(transaction)?,
            TransactionType::Chargeback => self.process_chargeback(transaction)?,
        };

        Ok(())
    }

    fn track_transaction(
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

    fn deposit_funds(account: &mut Account, amount: f64) {
        println!("depositing {} into account {}", amount, account.client);
        account.available += amount;
        account.total = account.available - account.held;
    }

    fn withdraw_funds(account: &mut Account, amount: f64) {
        println!("withdrawing {} from account {}", amount, account.client);
        account.available -= amount;
        account.total = account.available - account.held;
    }

    fn process_deposit(&mut self, transaction: &Transaction) -> Result<(), DepositError> {
        let amount = match transaction.amount {
            None => return Err(DepositError::AmountRequired),
            Some(amount) => amount,
        };
        self.track_transaction(&transaction)?;

        match self.accounts.entry(transaction.client) {
            Entry::Occupied(mut account) => {
                let account = account.get_mut();

                if account.locked {
                    return Err(DepositError::AccountLocked);
                }

                Self::deposit_funds(account, amount);
            }
            Entry::Vacant(vacancy) => {
                println!(
                    "initial deposit of {} into account {}",
                    amount, transaction.client
                );
                vacancy.insert(Account::new_account(transaction.client, amount));
            }
        };

        Ok(())
    }

    fn process_withdrawal(&mut self, transaction: &Transaction) -> Result<(), WithdrawalError> {
        let amount = match transaction.amount {
            None => return Err(WithdrawalError::AmountRequired),
            Some(amount) => amount,
        };
        self.track_transaction(&transaction)?;

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

                Self::withdraw_funds(account, amount);
            }
            Entry::Vacant(_) => return Err(WithdrawalError::NoSuchAccount(transaction.client)),
        };

        Ok(())
    }

    fn get_account(&mut self, id: u16) -> Result<OccupiedEntry<u16, Account>, AccountError> {
        match self.accounts.entry(id) {
            Entry::Occupied(account) => {
                if account.get().locked {
                    return Err(AccountError::AccountLocked(id));
                }

                Ok(account)
            }
            Entry::Vacant(_) => return Err(AccountError::NoSuchAccount(id)),
        }
    }

    fn process_dispute(&mut self, transaction: &Transaction) -> Result<(), DisputeError> {
        match self.transactions.entry(transaction.tx) {
            Entry::Occupied(mut tx) => {
                let tx = tx.get_mut();
                let amount = match tx.amount {
                    Some(amount) => amount,
                    None => return Err(DisputeError::AmountRequired),
                };

                let mut account = self.get_account(transaction.client)?;
                let account = account.get_mut();

                account.available -= amount;
                account.held += amount;
                self.disputed_transactions.insert(transaction.tx);
            }
            Entry::Vacant(_) => return Err(DisputeError::NoSuchTransaction(transaction.tx)),
        };

        Ok(())
    }

    fn process_resolve(&mut self, transaction: &Transaction) -> Result<(), ResolveError> {
        match self.transactions.entry(transaction.tx) {
            Entry::Occupied(tx) => {
                if !self.disputed_transactions.contains(&transaction.tx) {
                    return Err(ResolveError::TransactionNotDisputed(transaction.tx));
                }

                let tx = tx.get();
                let amount = match tx.amount {
                    Some(a) => a,
                    None => return Err(ResolveError::AmountRequired),
                };

                let mut account = self.get_account(transaction.client)?;
                let account = account.get_mut();

                account.available += amount;
                account.held -= amount;
                self.disputed_transactions.insert(transaction.tx);
            }
            Entry::Vacant(_) => {
                return Err(ResolveError::NoSuchTransaction(transaction.tx));
            }
        };
        Ok(())
    }

    fn process_chargeback(&mut self, transaction: &Transaction) -> Result<(), ChargebackError> {
        match self.transactions.entry(transaction.tx) {
            Entry::Occupied(tx) => {
                if !self.disputed_transactions.contains(&transaction.tx) {
                    return Err(ChargebackError::TransactionNotDisputed(transaction.tx));
                }

                let tx = tx.get();
                let amount = match tx.amount {
                    Some(a) => a,
                    None => return Err(ChargebackError::AmountRequired),
                };

                let mut account = self.get_account(transaction.client)?;
                let account = account.get_mut();
                account.held -= amount;
                account.total = account.available - account.held;
                account.locked = true;
            }
            Entry::Vacant(_) => {
                return Err(ChargebackError::NoSuchTransaction(transaction.tx));
            }
        };
        Ok(())
    }
}

// pub struct TxIter<I> {
//     iter: I,
// }
//
// impl<I> TxIter<I> {
//     pub fn new(iter: I) -> Self {
//         Self { iter }
//     }
//
//     pub fn open(path: &str) -> Result<Self, Box<dyn Error>> {
//         let mut reader = csv::ReaderBuilder::new()
//             .flexible(true)
//             .has_headers(true)
//             .trim(Trim::All)
//             .from_path(path)?;
//
//         Ok(Self::new(reader.deserialize()))
//     }
// }
//
// impl<I> Iterator for TxIter<I>
// where
//     I: Iterator,
// {
//     type Item = Transaction;
//
//     fn next(&mut self) -> Option<Self::Item> {
//         let _item = self.iter.next();
//
//         None
//     }
// }

// pub struct TransactionReader<R: io::Read> {
//     reader: csv::Reader<R>,
// }
//
// impl<R: io::Read> TransactionReader<R> {
//     pub fn iter(&mut self) -> TransactionIter<R, dyn DeserializeOwned> {
//         TransactionIter::new(&mut self.reader)
//     }
// }
//
// impl<R: io::Read> TryFrom<&str> for TransactionReader<R> {
//     type Error = Box<dyn Error>;
//
//     fn try_from(path: &str) -> Result<Self, Self::Error> {
//         Ok(Self {
//             reader: csv::ReaderBuilder::new().flexible(true).from_path(path)?,
//         })
//     }
// }
//
// pub struct TransactionIter<'r, R: 'r, D: DeserializeOwned> {
//     // data: csv::Reader<BufReader<File>>,
//     reader: csv::DeserializeRecordsIter<'r, R, D>,
//     rec: Transaction,
// }
//
// impl<'r, R: io::Read, D: DeserializeOwned> TransactionIter<'r, R, D> {
//     pub fn new(reader: &'r mut csv::Reader<R>) -> Self {
//         Self {
//             reader: reader.deserialize(),
//             rec: Transaction::new(),
//         }
//     }
// }
//
// impl<'r, R: io::Read, D: DeserializeOwned> Iterator for TransactionIter<'r, R, D> {
//     // TODO: turn this into a Result type so we can bubble up errors as needed
//     type Item = Result<Transaction, ()>;
//
//     fn next(&mut self) -> Option<Self::Item> {
//         self.data.deserialize()
//     }
// }

#[cfg(test)]
mod tests {
    use super::*;
}
