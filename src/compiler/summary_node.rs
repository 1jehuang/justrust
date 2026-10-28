//! SUMMARY's graph adapter and context-free summary computation.
use super::{Context, Error, Key, LOCAL_OPT, SsaFact, SummaryFact, envelope, ir, keys, ssa};
mod requests;

/// The only interpretation of a request from this producer is the same
/// instance's LOCAL_OPT output zero. The request protocol has no node-kind,
/// output-index, whole-program context, or graph handle capability.
pub(super) async fn produce(
    cx: &Context,
    key: &Key,
    budget: &ir::PlanBudget,
) -> Result<SummaryFact, Error> {
    let mut requests = requests::Requests::new(&key.params);
    let mut body = None;
    while let Some(instance) = requests.next() {
        let input = keys::words(LOCAL_OPT, instance.to_vec())?;
        body = Some(cx.read_fact::<SsaFact>(cx.request(input).get(0)).await?);
    }
    let body = body.ok_or_else(|| Error::new("summary protocol did not request its body"))?;
    summarize(body.as_ref(), budget)
}

/// Deliberately no Context, Facts, provider, or lookup callback: summary work
/// can inspect only the one immutable body and its own resource meter.
fn summarize(body: &SsaFact, budget: &ir::PlanBudget) -> Result<SummaryFact, Error> {
    let meter = ssa::meter::Meter::new(budget);
    let summary = match body.lowered.as_deref() {
        Some(lowered) => Some(ssa::inline::Summary::of(lowered, &meter).map_err(Error::new)?),
        None => None,
    };
    // Summary is Copy and owns no heap storage. Keep the same fact receipt as
    // before isolating the request protocol.
    let receipt = meter
        .lease(envelope::<SummaryFact>())
        .and_then(|lease| lease.seal(&meter))
        .map_err(Error::new)?;
    Ok(SummaryFact {
        summary,
        _receipt: receipt,
    })
}
