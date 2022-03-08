#[derive(Debug, Deserialize, PartialEq)]
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
    amount: f64,
}

#[derive(Debug, Serialize)]
pub struct Account {
    client: u16,
    available: f64,
    held: f64,
    total: f64,
    locked: bool,
}
