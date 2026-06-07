//! Hybrid retrieval: lexical/dense/graph/recency/trust signals, RRF fusion, query-class router, and the recall bundle.
//!
//! This milestone (M1.T03–T04) implements the lexical and dense [`signals`]: each
//! turns a query into a best-first ranked candidate list for reciprocal-rank fusion
//! (03 §1–§2). The graph, recency, and trust signals, fusion, the query-class
//! router, and the recall bundle land with their tasks.

mod error;
mod signals;

pub use error::RetrievalError;
pub use signals::{
    DenseRanking, RankedCandidate, Signal, SignalRanking, dense_ranking, lexical_ranking,
};
