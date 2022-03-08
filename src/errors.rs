use std::error::Error;
use std::fmt;
use std::fmt::Formatter;
use std::string::FromUtf8Error;

#[derive(Debug, PartialEq)]
pub struct DuplicateTransactionError {
    tx_id: u32,
}

impl DuplicateTransactionError {
    pub fn new(tx_id: u32) -> Self {
        Self { tx_id }
    }
}

#[derive(Debug, PartialEq)]
pub enum AccountError {
    AccountLocked(u16),
    NoSuchAccount(u16),
}

#[derive(Debug, PartialEq)]
pub enum DepositError {
    AmountRequired,
    AccountLocked,
    DuplicateTx(DuplicateTransactionError),
    NegativeDeposit,
}

#[derive(Debug)]
pub enum WithdrawalError {
    AmountRequired,
    AccountLocked,
    InsufficientFunds(f64, f64),
    NoSuchAccount(u16),
    DuplicateTx(DuplicateTransactionError),
    NegativeWithdrawal,
}

#[derive(Debug)]
pub enum DisputeError {
    AccountLocked,
    NoSuchAccount(u16),
    AmountRequired,
}

#[derive(Debug)]
pub enum ResolveError {
    AccountLocked,
    NoSuchAccount(u16),
    AmountRequired,
}

#[derive(Debug)]
pub enum ChargebackError {
    AccountLocked,
    NoSuchAccount(u16),
    AmountRequired,
}

#[derive(Debug)]
pub enum StatementError {
    SerializeError(csv::Error),
    EncodingError(FromUtf8Error),
}

impl fmt::Display for DuplicateTransactionError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "duplicate transaction id {} detected", self.tx_id)
    }
}

impl fmt::Display for AccountError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            AccountError::AccountLocked(id) => write!(f, "account {} is locked", id),
            AccountError::NoSuchAccount(id) => write!(f, "no such account: {}", id),
        }
    }
}

impl fmt::Display for DepositError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            DepositError::AmountRequired => write!(
                f,
                "deposit transactions MUST specify an amount, but none was provided"
            ),
            DepositError::AccountLocked => write!(f, "unable to deposit funds, account is locked"),
            DepositError::DuplicateTx(err) => write!(f, "failed to deposit funds: {}", err),
            DepositError::NegativeDeposit => {
                write!(f, "unable to deposit funds, amount is negative")
            }
        }
    }
}

impl fmt::Display for WithdrawalError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            WithdrawalError::AmountRequired => write!(
                f,
                "withdrawal transactions MUST specify an amount, but none was provided"
            ),
            WithdrawalError::AccountLocked => {
                write!(f, "unable to withdraw funds, account is locked")
            }
            WithdrawalError::InsufficientFunds(wanted, had) => {
                write!(
                    f,
                    "insufficient funds to complete this transaction wanted={} had={}",
                    wanted, had
                )
            }
            WithdrawalError::NoSuchAccount(account) => {
                write!(
                    f,
                    "unable to withdraw funds from non-existent account {}",
                    account
                )
            }
            WithdrawalError::DuplicateTx(err) => write!(f, "failed to withdraw funds: {}", err),
            WithdrawalError::NegativeWithdrawal => {
                write!(f, "unable to withdraw funds, amount is negative")
            }
        }
    }
}

impl fmt::Display for DisputeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            DisputeError::AccountLocked => {
                write!(f, "unable to dispute transaction, account is locked")
            }
            DisputeError::NoSuchAccount(id) => write!(
                f,
                "unable to dispute transaction with non-existent account {}",
                id
            ),
            DisputeError::AmountRequired => write!(
                f,
                "disputed transactions MUST have a specified amount, but none was present"
            ),
        }
    }
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ResolveError::AccountLocked => {
                write!(f, "unable to resolve transaction, account is locked")
            }
            ResolveError::NoSuchAccount(id) => write!(
                f,
                "unable to resolve disputed transaction with non-existent account {}",
                id
            ),
            ResolveError::AmountRequired => write!(
                f,
                "transactions MUST have a specified amount in order to be resolved"
            ),
        }
    }
}

impl fmt::Display for ChargebackError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ChargebackError::AccountLocked => {
                write!(f, "unable to chargeback transaction, account is locked")
            }
            ChargebackError::NoSuchAccount(id) => write!(
                f,
                "unable to charge back transaction with non-existent account: {}",
                id
            ),
            ChargebackError::AmountRequired => write!(
                f,
                "transactions MUST have a specified amount in order to be charged back"
            ),
        }
    }
}

impl fmt::Display for StatementError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            StatementError::SerializeError(err) => {
                write!(f, "failed to serialize statement record: {}", err)
            }
            StatementError::EncodingError(err) => write!(f, "failed to encode csv report: {}", err),
        }
    }
}

impl From<DuplicateTransactionError> for DepositError {
    fn from(err: DuplicateTransactionError) -> Self {
        DepositError::DuplicateTx(err)
    }
}

impl From<DuplicateTransactionError> for WithdrawalError {
    fn from(err: DuplicateTransactionError) -> Self {
        WithdrawalError::DuplicateTx(err)
    }
}

impl From<AccountError> for DisputeError {
    fn from(err: AccountError) -> Self {
        match err {
            AccountError::AccountLocked(_) => DisputeError::AccountLocked,
            AccountError::NoSuchAccount(id) => DisputeError::NoSuchAccount(id),
        }
    }
}

impl From<AccountError> for ResolveError {
    fn from(err: AccountError) -> Self {
        match err {
            AccountError::AccountLocked(_) => ResolveError::AccountLocked,
            AccountError::NoSuchAccount(id) => ResolveError::NoSuchAccount(id),
        }
    }
}

impl From<AccountError> for ChargebackError {
    fn from(err: AccountError) -> Self {
        match err {
            AccountError::AccountLocked(_) => ChargebackError::AccountLocked,
            AccountError::NoSuchAccount(id) => ChargebackError::NoSuchAccount(id),
        }
    }
}

impl From<csv::Error> for StatementError {
    fn from(err: csv::Error) -> Self {
        StatementError::SerializeError(err)
    }
}

impl From<FromUtf8Error> for StatementError {
    fn from(err: FromUtf8Error) -> Self {
        StatementError::EncodingError(err)
    }
}

impl Error for DuplicateTransactionError {}
impl Error for AccountError {}
impl Error for DepositError {}
impl Error for WithdrawalError {}
impl Error for DisputeError {}
impl Error for ResolveError {}
impl Error for ChargebackError {}
impl Error for StatementError {}
