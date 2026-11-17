//! Transfer exactly $1 from Predictions to Perps, retaining IDs for safe reconciliation.
use kalshi_sdk_rs::{
    margin::models::{
        ExchangeInstance as MarginExchange, GetMarginBalanceQuery,
        IntraExchangeInstanceTransferRequest,
    },
    models::{
        ExchangeInstance, GetBalanceQuery, GetIntraExchangeInstanceTransfersQuery,
        IntraExchangeInstanceTransfer, IntraExchangeInstanceTransferStatus,
    },
    Client, Credentials, Environment, Error as SdkError,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Debug, Serialize, Deserialize)]
struct TransferState {
    environment: String,
    client_transfer_id: String,
    transfer_id: Option<String>,
}

fn write_state(path: &Path, state: &TransferState, initial: bool) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let destination = if initial { path } else { &temporary };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    serde_json::to_writer_pretty(&mut file, state)?;
    writeln!(file)?;
    file.sync_all()?;
    drop(file);
    if !initial {
        fs::rename(&temporary, path)?;
    }
    Ok(())
}

fn load_state(path: &Path, environment: &str) -> Result<TransferState> {
    let state = if path.exists() {
        serde_json::from_slice(&fs::read(path)?)?
    } else {
        let state = TransferState {
            environment: environment.into(),
            client_transfer_id: uuid::Uuid::new_v4().to_string(),
            transfer_id: None,
        };
        write_state(path, &state, true)?;
        state
    };
    if state.environment != environment {
        return Err(format!("state belongs to {}, but KALSHI_ENV selects {environment}; use the matching environment or a different --state path", state.environment).into());
    }
    uuid::Uuid::parse_str(&state.client_transfer_id)?;
    Ok(state)
}

fn validate_transfer(
    transfer: &IntraExchangeInstanceTransfer,
    state: &TransferState,
) -> Result<()> {
    if transfer.source != ExchangeInstance::EventContract
        || transfer.destination != ExchangeInstance::Margined
        || transfer.source_exchange_shard != 0
        || transfer.destination_exchange_shard != 0
        || transfer.amount.decimal() != Decimal::ONE
        || state
            .transfer_id
            .as_ref()
            .is_some_and(|id| id != &transfer.transfer_id)
        || transfer
            .client_transfer_id
            .as_ref()
            .is_some_and(|id| id != &state.client_transfer_id)
    {
        return Err("the recorded transfer does not match this $1 Predictions-to-Perps request; no new transfer submitted".into());
    }
    Ok(())
}

async fn find_transfer(client: &Client, state: &TransferState) -> Result<Option<String>> {
    let mut cursor = None;
    for _ in 0..20 {
        let page = client
            .get_intra_exchange_instance_transfers(&GetIntraExchangeInstanceTransfersQuery {
                limit: Some(500),
                cursor,
            })
            .await?;
        if let Some(transfer) = page
            .transfers
            .iter()
            .find(|t| t.client_transfer_id.as_deref() == Some(&state.client_transfer_id))
        {
            validate_transfer(transfer, state)?;
            return Ok(Some(transfer.transfer_id.clone()));
        }
        cursor = page.next_cursor().map(str::to_owned);
        if cursor.is_none() {
            return Ok(None);
        }
    }
    Err("transfer history exceeded 20 pages; reconcile the saved client_transfer_id before submitting again".into())
}

async fn print_balances(client: &Client) -> Result<Decimal> {
    let predictions = client
        .get_balance(&GetBalanceQuery {
            subaccount: Some(0),
            exchange_index: Some(0),
        })
        .await?;
    println!(
        "Predictions primary account, shard 0: ${} available",
        predictions.balance_dollars
    );
    match client
        .margin()
        .get_balance(&GetMarginBalanceQuery {
            compute_available_balance: Some(true),
        })
        .await
    {
        Ok(balance) => {
            if let Some(settled) = balance.settled_funds {
                println!("Perps settled funds: ${settled}");
            }
            for account in balance.subaccount_balances {
                println!(
                    "Perps subaccount {} ({}): equity=${}, available=${}",
                    account.subaccount,
                    account.subaccount_type.as_str(),
                    account.account_equity,
                    account.available_balance
                );
            }
        }
        Err(error) => eprintln!("Perps balance could not be read: {error}"),
    }
    Ok(predictions.balance_dollars.decimal())
}

async fn wait_for_transfer(client: &Client, state: &TransferState, wait: Duration) -> Result<()> {
    let transfer_id = state.transfer_id.as_deref().ok_or("missing transfer ID")?;
    tokio::time::timeout(wait, async {
        loop {
            match client.get_intra_exchange_instance_transfer(transfer_id).await {
                Ok(response) => {
                    validate_transfer(&response.transfer, state)?;
                    println!("Transfer {transfer_id}: {}", response.transfer.status.as_str());
                    match response.transfer.status {
                        IntraExchangeInstanceTransferStatus::Complete => return Ok(()),
                        IntraExchangeInstanceTransferStatus::Pending => {}
                        IntraExchangeInstanceTransferStatus::Unknown(status) => return Err(format!("unrecognized transfer status {status}; inspect transfer {transfer_id} before further action").into()),
                    }
                }
                // The asynchronous transfer's read model can lag its creation response.
                Err(SdkError::Api { status: 404, .. }) => {}
                Err(error) => return Err(format!("could not read transfer {transfer_id}: {error}").into()),
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }).await.map_err(|_| format!("transfer {transfer_id} is not confirmed complete after {} seconds; retain this state file and rerun to check it without creating another transfer", wait.as_secs()))?
}

async fn transfer_one_dollar(
    client: &Client,
    state: &mut TransferState,
    state_path: &Path,
    wait: Duration,
) -> Result<()> {
    // Also persist states supplied programmatically before any money-moving request.
    if !state_path.exists() {
        write_state(state_path, state, true)?;
    }
    println!("Client transfer ID: {}", state.client_transfer_id);
    if state.transfer_id.is_none() {
        state.transfer_id = find_transfer(client, state).await?;
        if state.transfer_id.is_some() {
            write_state(state_path, state, false)?;
        }
    }
    if state.transfer_id.is_none() {
        if print_balances(client).await? < Decimal::ONE {
            return Err("less than $1 is available in Predictions primary account/shard 0; no transfer submitted".into());
        }
        println!("Submitting exactly $1.00 (10,000 centicents): Predictions -> Perps");
        let result = client
            .margin()
            .intra_exchange_instance_transfer(&IntraExchangeInstanceTransferRequest {
                client_transfer_id: Some(state.client_transfer_id.clone()),
                source: MarginExchange::EventContract,
                destination: MarginExchange::Margined,
                amount: 10_000,
                source_exchange_shard: Some(0),
                destination_exchange_shard: Some(0),
                ..Default::default()
            })
            .await;
        match result {
            Ok(response) => state.transfer_id = Some(response.transfer_id),
            Err(error) => {
                let uncertain = matches!(&error, SdkError::Http(_) | SdkError::Json(_))
                    || matches!(&error, SdkError::Api { status, .. } if *status == 409 || *status >= 500);
                if !uncertain {
                    return Err(error.into());
                }
                // Reconcile once; never replay a transfer POST automatically.
                match find_transfer(client, state).await {
                    Ok(Some(id)) => state.transfer_id = Some(id),
                    Ok(None) => return Err(format!("{error}; transfer outcome is unconfirmed. Retain the state file/client_transfer_id and reconcile before retrying").into()),
                    Err(lookup) => return Err(format!("{error}; transfer reconciliation also failed: {lookup}. Retain the state file; do not generate a new client_transfer_id for a retry").into()),
                }
            }
        }
        println!(
            "Transfer accepted: {}",
            state.transfer_id.as_deref().unwrap()
        );
        write_state(state_path, state, false).map_err(|error| format!("transfer {} was accepted, but its ID could not be saved: {error}; retain client_transfer_id {}", state.transfer_id.as_deref().unwrap(), state.client_transfer_id))?;
    } else {
        println!(
            "Resuming transfer {}; no transfer POST will be sent",
            state.transfer_id.as_deref().unwrap()
        );
    }
    wait_for_transfer(client, state, wait).await?;
    println!("Transfer confirmed complete. Current balances:");
    if let Err(error) = print_balances(client).await {
        eprintln!("Transfer is complete, but balances could not be refreshed: {error}");
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut submit = false;
    let mut state_path = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--submit" => submit = true,
            "--state" => {
                state_path = Some(PathBuf::from(args.next().ok_or("--state needs a path")?))
            }
            "--help" | "-h" => {
                println!("Usage: cargo run --example margin_transfer -- --submit [--state PATH]\nTransfers exactly $1 from Predictions primary account/shard 0 to Perps primary account.\nKALSHI_ENV selects demo (default) or production; existing API key variables are required.\nThe default target/margin-transfer-one-dollar-<environment>.json retains IDs. Reuse it to resume; a fresh --state path starts a separate $1 transfer. Status checks wait up to 15 seconds.");
                return Ok(());
            }
            _ => return Err(format!("unknown argument {arg}; use --help").into()),
        }
    }
    if !submit {
        return Err(
            "pass --submit to explicitly request the $1 transfer; use --help for details".into(),
        );
    }
    let (environment, name) = match std::env::var("KALSHI_ENV").as_deref() {
        Ok("demo") | Err(std::env::VarError::NotPresent) => (Environment::Demo, "demo"),
        Ok("production") => (Environment::Production, "production"),
        _ => return Err("KALSHI_ENV must be demo or production".into()),
    };
    let client = Client::builder(environment)
        .credentials(Credentials::from_env()?)
        .timeout(Duration::from_secs(15))
        .build()?;
    let state_path = state_path
        .unwrap_or_else(|| PathBuf::from(format!("target/margin-transfer-one-dollar-{name}.json")));
    let mut state = load_state(&state_path, name)?;
    println!(
        "Environment: {environment:?}; state: {}",
        state_path.display()
    );
    transfer_one_dollar(&client, &mut state, &state_path, Duration::from_secs(15))
        .await
        .map_err(|error| {
            format!(
                "{error}; state: {} (reuse this path to resume the same request)",
                state_path.display()
            )
            .into()
        })
}

#[cfg(test)]
#[path = "common/margin_transfer_tests.rs"]
mod tests;
