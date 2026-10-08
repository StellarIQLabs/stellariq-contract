# Donations contract

`contracts/donations` is the core of StellarIQ Give. It lets a charity open a
fundraising campaign and lets anyone donate to it with full on-chain
transparency.

- **Testnet:** `CBCHKIDRFJ4KO2DGJEP75NJPYN65YVD6QOVVHC5IU7PRTHGHW75OF2KX`
  ([stellar.expert](https://stellar.expert/explorer/testnet/contract/CBCHKIDRFJ4KO2DGJEP75NJPYN65YVD6QOVVHC5IU7PRTHGHW75OF2KX))
- **Accepted token (demo):** native XLM SAC `CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC`

## How it works

1. A campaign is created with a beneficiary wallet, the token it accepts, a
   goal and a deadline.
2. A donor calls `donate`. The token moves from the donor **directly** to the
   beneficiary inside the same transaction. The contract never holds funds.
3. The contract updates the campaign totals, stores a `Receipt` and emits a
   `donation_made` event that the app and indexer read.
4. To give a different asset, the app first converts it with the StellarIQ
   swap router, then calls `donate` with the campaign token.

## Entry points

| Function | Auth | Notes |
|---|---|---|
| `__constructor(admin)` | deployer | runs once at deploy |
| `create_campaign(creator, beneficiary, token, title, description, goal, deadline)` | creator | returns campaign id |
| `donate(donor, campaign_id, amount, memo)` | donor | returns receipt id |
| `close_campaign(caller, id)` | creator or admin | stops new donations |
| `set_paused(paused)` | admin | emergency stop |
| `get_campaign`, `campaign_count`, `get_receipt`, `receipt_count`, `donor_total`, `is_paused`, `admin_address` | none | read only |

## Events

| Event | Topics | Data |
|---|---|---|
| `campaign_created` | `campaign_id` | creator, beneficiary, token, goal, deadline, version |
| `donation_made` | `campaign_id`, `donor` | receipt_id, amount, raised, version |
| `campaign_closed` | `campaign_id` | raised, version |

## Try it from the CLI

```sh
ID=CBCHKIDRFJ4KO2DGJEP75NJPYN65YVD6QOVVHC5IU7PRTHGHW75OF2KX
stellar keys generate me --network testnet --fund
stellar contract invoke --id $ID --network testnet --source me -- \
  donate --donor $(stellar keys address me) --campaign_id 1 \
  --amount 100000000 --memo "hello from the CLI"
stellar contract invoke --id $ID --network testnet --source me --send=no -- get_campaign --id 1
```
