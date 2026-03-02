# GitHub Copilot Instructions for RainySim

## PRIMARY DIRECTIVE: READ-ONLY MODE BY DEFAULT

**MUST READ AND FOLLOW:** Under no circumstances should you ever propose, write, or apply any code changes, edits, or refactorings to any file unless I explicitly and directly ask you to "change", "edit", "refactor", "fix", or "implement" something. Your primary role is to answer questions, provide explanations, and act as a read-only assistant. If I ask a question, provide an answer without modifying code.

---

## VERSION COMPATIBILITY
You must ensure all suggestions and code snippets are fully compatible with the crate versions actually used as found in Cargo.toml. This especially
includes polars, making sure to use the applicable API and avoid older
deprecated methods and patterns. 

---

## Project Overview
RainySim is a high-performance trade simulator built in pure Rust. It consists of modular components for data feeds, trading models, and simulation engines with a focus on functional programming patterns and performance.

## Core Architecture
- `datafeed/`: Data source abstractions (daily/intraday market data)
- `model/`: Trading models and prediction engines
- `simulator/`: Core simulation engine and orchestration
- Uses trait-based design with `Box<dyn Trait>` for polymorphism

## Coding Standards & Preferences

### 1. Idiomatic Rust - HIGHEST PRIORITY
- Use Rust's ownership system naturally - prefer borrowing over cloning
- Leverage the type system for compile-time guarantees
- Use `Result<T, E>` for error handling, never panic in library code
- Prefer explicit error types using `thiserror` crate
- Use pattern matching extensively with `match` and `if let`
- Follow Rust naming conventions (snake_case for variables/functions, PascalCase for types)

### 2. Immutability Preference - HIGH PRIORITY
- Default to immutable bindings (`let` not `let mut`)
- Only use `mut` when mutation is truly necessary
- Prefer creating new values over mutating existing ones when reasonable
- Use `Cow<'_, T>` when you might need owned or borrowed data
- Design APIs to minimize required mutations

### 3. Performance Consciousness - HIGH PRIORITY
- Avoid unnecessary allocations - prefer borrowing and slicing
- Use `&str` over `String` for function parameters when possible
- Prefer `Vec` over `LinkedList`, `HashMap` over `BTreeMap` unless ordering needed
- Consider using `SmallVec` for small collections
- Use iterator adapters which are zero-cost abstractions
- Profile-guided optimization - measure before optimizing

### 4. Functional Programming Approach - CRITICAL REQUIREMENT
**ALWAYS prefer functional iterator methods over imperative loops:**

#### ✅ PREFERRED Functional Patterns:
```rust
// Use map for transformations
let doubled: Vec<i32> = numbers.iter().map(|x| x * 2).collect();

// Use filter for conditions
let evens: Vec<i32> = numbers.iter().filter(|&x| x % 2 == 0).collect();

// Use filter_map to combine filter + map
let results: Vec<f64> = strings.iter()
    .filter_map(|s| s.parse().ok())
    .collect();

// Use flat_map for flattening
let all_items: Vec<Item> = groups.iter()
    .flat_map(|group| group.items.iter())
    .collect();

// Chain multiple operations
let processed: Vec<_> = data.iter()
    .filter(|item| item.is_valid())
    .map(|item| item.transform())
    .collect();

// Use find for searching
let result = items.iter().find(|item| item.matches_criteria());

// Use any/all for boolean checks
let has_valid = items.iter().any(|item| item.is_valid());
let all_valid = items.iter().all(|item| item.is_valid());
```

#### ❌ AVOID These Patterns:
```rust
// DON'T use for loops when functional methods work
for item in items {
    // ... manual processing
}

// DON'T use fold when you're just doing imperative accumulation
let result = items.iter().fold(Vec::new(), |mut acc, item| {
    // This is just a disguised for-loop!
    acc.push(process(item));
    acc
});
```

#### ✅ Proper use of `fold` (accumulation to single value):
```rust
// Sum/product operations
let sum = numbers.iter().fold(0, |acc, &x| acc + x);
// Or better: let sum: i32 = numbers.iter().sum();

// Building complex accumulator
let stats = data.iter().fold(Stats::new(), |stats, item| {
    stats.add_observation(item.value)
});
```

### 5. Error Handling Standards
- Use `thiserror` for custom error types
- Implement proper error context with descriptive messages
- Use `?` operator for error propagation
- Define module-specific error enums (e.g., `DataFeedError`, `PredictionError`)
- Never use `unwrap()` or `expect()` in production code paths

### 6. Documentation Standards
- Document all public APIs with `///` comments
- Include examples in documentation when helpful
- Use `#[doc(hidden)]` for internal APIs
- Document safety requirements for any `unsafe` code
- Include module-level documentation explaining purpose and usage
- Use rust conventions for markdown styles, e.g. dashes for unordered lists.
- Use rust markdown extensions where applicable, e.g. links to types with backticks.

### 7. Testing Standards
- Write unit tests for all public functions
- Use property-based testing for complex logic
- Prefer `assert_eq!` over `assert!` for better error messages
- Use `#[should_panic]` judiciously with specific expected messages
- Test error conditions, not just happy paths

### 8. Trait Design
- Prefer small, focused traits over large ones
- Use associated types over generics when there's one logical type choice
- Provide default implementations when reasonable
- Use `Box<dyn Trait>` for heterogeneous collections (already used in project)
- Consider object safety when designing traits

### 9. Module Organization
- Keep modules focused and cohesive
- Re-export public APIs at appropriate levels
- Use `pub(crate)` for internal APIs
- Organize by feature, not by technical layer when possible

### 10. Import Statement Guidelines - CRITICAL

Import paths follow three context-specific rules. Apply the correct rule based on where the file lives.

#### Rule 1 — Library source files (`src/**/*.rs`)

Stop at the `orderbook` module re-export layer (`orderbook/mod.rs`). For types that are *not* re-exported from `mod.rs` (e.g. `OrderLevel`), go one level deeper to the defining module.

```rust
// ✅ CORRECT — stop at orderbook module re-export layer
use crate::orderbook::{Order, OrderBook, MboObserver, MboProcessor, TradeEvent};
use crate::orderbook::book::OrderLevel;  // not re-exported, so direct module path is fine

// ❌ WRONG — super:: for sibling references (fragile, unclear)
use super::book::OrderBook;
use super::mbo::MboProcessor;

// ❌ WRONG — crate:: root re-exports inside library code
use crate::{Order, OrderBook};
```

`super::` is only appropriate for a child module accessing its direct parent. Never use `super::` for sibling-to-sibling references.

#### Rule 2 — Test blocks within library source files

`#[cfg(test)] mod tests` uses `use super::*;` to import the module under test. All `use` statements must be at the top of the `mod tests` block — never inside individual `#[test]` functions.

```rust
#[cfg(test)]
mod tests {
    use super::*;  // import module under test
    use time::{Duration, OffsetDateTime};
    use crate::orderbook::mbo::{Action, MarketByOrderMessage, MboProcessor};
    // All use statements here at top — NOT inside individual test functions
}
```

#### Rule 3 — Binaries, examples, benchmarks

These are separate crate contexts. Use the library's public crate name and root re-exports for all public types.

```rust
// ✅ CORRECT — crate name + root re-exports for public types
use rainybook::{Order, OrderBook, Side, MboProcessor};

// ✅ CORRECT — submodule path only when NOT re-exported at root
use rainybook::generators::OrderGenerator;

// ❌ WRONG — accessing internal module paths
use rainybook::orderbook::book::Order;
use rainybook::orderbook::Order;
```

#### Summary

| Context | Correct pattern | Wrong patterns |
|---|---|---|
| Library `src/**/*.rs` | `crate::orderbook::Type` | `super::sibling::Type`, `crate::Type` (root) |
| Test `mod tests` | `use super::*;` at top of block | `use` inside individual test functions |
| Bins / examples / benches | `rainybook::Type` | `rainybook::orderbook::Type` |

#### Import grouping and organization:
```rust
// 1. Standard library imports
use std::{fs, io};

// 2. External crate imports
use time::{Duration, OffsetDateTime};
use thiserror::Error;

// 3. Internal crate imports (use correct rule per context above)
use crate::orderbook::{Order, OrderBook};
```

**Key principle:** Always import types at the top of the file. Never use inline `::` paths in type signatures, struct fields, or function signatures.

### 11. Output and Logging
- **Binaries in `src/`** must use `tracing` and `tracing_subscriber` for all diagnostic output (info, debug, warnings, errors). Do not use `println!` or `eprintln!` for logging.
  - The exception is output that is part of the program's actual user-facing output (e.g. displaying results to stdout for user consumption or interaction).
- **Examples in `examples/`** may use `println!` freely for short, illustrative output. These are exploratory programs and do not require the tracing infrastructure.

### 12. Dependencies and Features
- Minimize dependencies - prefer std library when sufficient
- Use feature flags for optional functionality
- Prefer well-maintained, widely-used crates
- Pin major versions in Cargo.toml

## Model Design for Python Interoperability
**Only consider when explicitly requested:**
- Design models as pure functions when possible
- Minimize Rust-specific types in model interfaces
- Consider using `pyo3` for Python bindings
- Use JSON or similar for data exchange when needed
- Keep model state serializable

## Code Examples

### Preferred Function Signature Style:
```rust
// Good: Borrows input, clear ownership
fn process_data(input: &[DataPoint]) -> Result<Vec<ProcessedData>, ProcessingError>

// Good: Builder pattern for complex construction
impl TradeSimulator {
    pub fn builder() -> TradeSimulatorBuilder { ... }
}
```

### Preferred Error Handling:
```rust
#[derive(Debug, Error)]
pub enum SimulationError {
    #[error("Invalid time range: {start} to {end}")]
    InvalidTimeRange { start: DateTime<Utc>, end: DateTime<Utc> },
    
    #[error("Data feed error: {source}")]
    DataFeedError { #[from] source: DataFeedError },
}
```

### Preferred Iterator Chains:
```rust
// Transform model predictions into results
let results: Vec<_> = models.values()
    .map(|model| model.get_subscriptions())
    .flatten()
    .filter(|sub| data_feeds.contains_key(&sub.data_source))
    .map(|sub| process_subscription(sub))
    .collect::<Result<Vec<_>, _>>()?;
```

## Anti-patterns to Avoid
1. Using `clone()` unnecessarily instead of borrowing
2. Manual memory management when Rust's ownership works
3. Using `unwrap()` in production code
4. Imperative loops when functional methods are clearer
5. Over-engineering with complex generics when simple types suffice
6. Ignoring Clippy warnings without good reason

Remember: Code should be performant, readable, and leverage Rust's strengths. When in doubt, choose the more functional approach with iterator chains over manual loops.
