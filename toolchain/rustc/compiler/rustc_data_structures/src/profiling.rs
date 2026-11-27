//! # Rust Compiler Self-Profiling
//!
//! This module implements the basic framework for the compiler's self-
//! profiling support. It provides the `SelfProfiler` type which enables
//! recording "events". An event is something that starts and ends at a given
//! point in time and has an ID and a kind attached to it. This allows for
//! tracing the compiler's activity.
//!
//! Internally this module uses the custom tailored [measureme][mm] crate for
//! efficiently recording events to disk in a compact format that can be
//! post-processed and analyzed by the suite of tools in the `measureme`
//! project. The highest priority for the tracing framework is on incurring as
//! little overhead as possible.
//!
//!
//! ## Event Overview
//!
//! Events have a few properties:
//!
//! - The `event_kind` designates the broad category of an event (e.g. does it
//!   correspond to the execution of a query provider or to loading something
//!   from the incr. comp. on-disk cache, etc).
//! - The `event_id` designates the query invocation or function call it
//!   corresponds to, possibly including the query key or function arguments.
//! - Each event stores the ID of the thread it was recorded on.
//! - The timestamp stores beginning and end of the event, or the single point
//!   in time it occurred at for "instant" events.
//!
//!
//! ## Event Filtering
//!
//! Event generation can be filtered by event kind. Recording all possible
//! events generates a lot of data, much of which is not needed for most kinds
//! of analysis. So, in order to keep overhead as low as possible for a given
//! use case, the `SelfProfiler` will only record the kinds of events that
//! pass the filter specified as a command line argument to the compiler.
//!
//!
//! ## `event_id` Assignment
//!
//! As far as `measureme` is concerned, `event_id`s are just strings. However,
//! it would incur too much overhead to generate and persist each `event_id`
//! string at the point where the event is recorded. In order to make this more
//! efficient `measureme` has two features:
//!
//! - Strings can share their content, so that re-occurring parts don't have to
//!   be copied over and over again. One allocates a string in `measureme` and
//!   gets back a `StringId`. This `StringId` is then used to refer to that
//!   string. `measureme` strings are actually DAGs of string components so that
//!   arbitrary sharing of substrings can be done efficiently. This is useful
//!   because `event_id`s contain lots of redundant text like query names or
//!   def-path components.
//!
//! - `StringId`s can be "virtual" which means that the client picks a numeric
//!   ID according to some application-specific scheme and can later make that
//!   ID be mapped to an actual string. This is used to cheaply generate
//!   `event_id`s while the events actually occur, causing little timing
//!   distortion, and then later map those `StringId`s, in bulk, to actual
//!   `event_id` strings. This way the largest part of the tracing overhead is
//!   localized to one contiguous chunk of time.
//!
//! How are these `event_id`s generated in the compiler? For things that occur
//! infrequently (e.g. "generic activities"), we just allocate the string the
//! first time it is used and then keep the `StringId` in a hash table. This
//! is implemented in `SelfProfiler::get_or_alloc_cached_string()`.
//!
//! For queries it gets more interesting: First we need a unique numeric ID for
//! each query invocation (the `QueryInvocationId`). This ID is used as the
//! virtual `StringId` we use as `event_id` for a given event. This ID has to
//! be available both when the query is executed and later, together with the
//! query key, when we allocate the actual `event_id` strings in bulk.
//!
//! We could make the compiler generate and keep track of such an ID for each
//! query invocation but luckily we already have something that fits all the
//! the requirements: the query's `DepNodeIndex`. So we use the numeric value
//! of the `DepNodeIndex` as `event_id` when recording the event and then,
//! just before the query context is dropped, we walk the entire query cache
//! (which stores the `DepNodeIndex` along with the query key for each
//! invocation) and allocate the corresponding strings together with a mapping
//! for `DepNodeIndex as StringId`.
//!
//! [mm]: https://github.com/rust-lang/measureme/

use std::borrow::Borrow;
use std::collections::hash_map::Entry;
use std::error::Error;
use std::fmt::Display;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use std::{fs, hint, process};

pub use measureme::EventId;
use measureme::{EventIdBuilder, Profiler, SerializableString, StringId};
use parking_lot::RwLock;
use smallvec::SmallVec;
use tracing::warn;

use crate::fx::FxHashMap;
use crate::outline;
use crate::sync::AtomicU64;

bitflags::bitflags! {
    #[derive(Clone, Copy)]
    struct EventFilter: u16 {
        const GENERIC_ACTIVITIES  = 1 << 0;
        const QUERY_PROVIDERS     = 1 << 1;
        /// Store detailed instant events, including timestamp and thread ID,
        /// per each query cache hit. Note that this is quite expensive.
        const QUERY_CACHE_HITS    = 1 << 2;
        const QUERY_BLOCKED       = 1 << 3;
        const INCR_CACHE_LOADS    = 1 << 4;

        const QUERY_KEYS          = 1 << 5;
        const FUNCTION_ARGS       = 1 << 6;
        const LLVM                = 1 << 7;
        const INCR_RESULT_HASHING = 1 << 8;
        const ARTIFACT_SIZES      = 1 << 9;
        /// Store aggregated counts of cache hits per query invocation.
        const QUERY_CACHE_HIT_COUNTS  = 1 << 10;

        // keep this in sync with the `-Z self-profile-events` help message in rustc_session/src/options.rs
        const DEFAULT = Self::GENERIC_ACTIVITIES.bits() |
                        Self::QUERY_PROVIDERS.bits() |
                        Self::QUERY_BLOCKED.bits() |
                        Self::INCR_CACHE_LOADS.bits() |
                        Self::INCR_RESULT_HASHING.bits() |
                        Self::ARTIFACT_SIZES.bits() |
                        Self::QUERY_CACHE_HIT_COUNTS.bits();

        // keep this in sync with the `-Z self-profile-events` help message in rustc_session/src/options.rs
        const ARGS = Self::QUERY_KEYS.bits() | Self::FUNCTION_ARGS.bits();
        const QUERY_CACHE_HIT_COMBINED = Self::QUERY_CACHE_HITS.bits() | Self::QUERY_CACHE_HIT_COUNTS.bits();
    }
}

// keep this in sync with the `-Z self-profile-events` help message in rustc_session/src/options.rs
const EVENT_FILTERS_BY_NAME: &[(&str, EventFilter)] = &[
    ("none", EventFilter::empty()),
    ("all", EventFilter::all()),
    ("default", EventFilter::DEFAULT),
    ("generic-activity", EventFilter::GENERIC_ACTIVITIES),
    ("query-provider", EventFilter::QUERY_PROVIDERS),
    ("query-cache-hit", EventFilter::QUERY_CACHE_HITS),
    ("query-cache-hit-count", EventFilter::QUERY_CACHE_HIT_COUNTS),
    ("query-blocked", EventFilter::QUERY_BLOCKED),
    ("incr-cache-load", EventFilter::INCR_CACHE_LOADS),
    ("query-keys", EventFilter::QUERY_KEYS),
    ("function-args", EventFilter::FUNCTION_ARGS),
    ("args", EventFilter::ARGS),
    ("llvm", EventFilter::LLVM),
    ("incr-result-hashing", EventFilter::INCR_RESULT_HASHING),
    ("artifact-sizes", EventFilter::ARTIFACT_SIZES),
];

/// Something that uniquely identifies a query invocation.
pub struct QueryInvocationId(pub u32);

/// Which format to use for `-Z time-passes`
#[derive(Clone, Copy, PartialEq, Hash, Debug)]
pub enum TimePassesFormat {
    /// Emit human readable text
    Text,
    /// Emit structured JSON
    Json,
}

/// A reference to the SelfProfiler. It can be cloned and sent across thread
/// boundaries at will.
#[derive(Clone)]
pub struct SelfProfilerRef {
    // This field is `None` if self-profiling is disabled for the current
    // compilation session.
    profiler: Option<Arc<SelfProfiler>>,

    // We store the filter mask directly in the reference because that doesn't
    // cost anything and allows for filtering with checking if the profiler is
    // actually enabled.
    event_filter_mask: EventFilter,

    // Print verbose generic activities to stderr.
    print_verbose_generic_activities: Option<TimePassesFormat>,
}

impl SelfProfilerRef {
    pub fn new(
        profiler: Option<Arc<SelfProfiler>>,
        print_verbose_generic_activities: Option<TimePassesFormat>,
    ) -> SelfProfilerRef {
        // If there is no SelfProfiler then the filter mask is set to NONE,
        // ensuring that nothing ever tries to actually access it.
        let event_filter_mask =
            profiler.as_ref().map_or(EventFilter::empty(), |p| p.event_filter_mask);

        SelfProfilerRef { profiler, event_filter_mask, print_verbose_generic_activities }
    }

    /// This shim makes sure that calls only get executed if the filter mask
    /// lets them pass. It also contains some trickery to make sure that
    /// code is optimized for non-profiling compilation sessions, i.e. anything
    /// past the filter check is never inlined so it doesn't clutter the fast
    /// path.
    #[inline(always)]
    fn exec<F>(&self, event_filter: EventFilter, f: F) -> TimingGuard<'_>
    where
        F: for<'a> FnOnce(&'a SelfProfiler) -> TimingGuard<'a>,
    {
        #[inline(never)]
        #[cold]
        fn cold_call<F>(profiler_ref: &SelfProfilerRef, f: F) -> TimingGuard<'_>
        where
            F: for<'a> FnOnce(&'a SelfProfiler) -> TimingGuard<'a>,
        {
            let profiler = profiler_ref.profiler.as_ref().unwrap();
            f(profiler)
        }

        if self.event_filter_mask.contains(event_filter) {
            cold_call(self, f)
        } else {
            TimingGuard::none()
        }
    }

    /// Start profiling a verbose generic activity. Profiling continues until the
    /// VerboseTimingGuard returned from this call is dropped. In addition to recording
    /// a measureme event, "verbose" generic activities also print a timing entry to
    /// stderr if the compiler is invoked with -Ztime-passes.
    pub fn verbose_generic_activity(&self, event_label: &'static str) -> VerboseTimingGuard<'_> {
        let message_and_format =
            self.print_verbose_generic_activities.map(|format| (event_label.to_owned(), format));

        VerboseTimingGuard::start(message_and_format, self.generic_activity(event_label))
            .with_jr_phase(event_label)
    }

    /// Like `verbose_generic_activity`, but with an extra arg.
    pub fn verbose_generic_activity_with_arg<A>(
        &self,
        event_label: &'static str,
        event_arg: A,
    ) -> VerboseTimingGuard<'_>
    where
        A: Borrow<str> + Into<String>,
    {
        let message_and_format = self
            .print_verbose_generic_activities
            .map(|format| (format!("{}({})", event_label, event_arg.borrow()), format));

        VerboseTimingGuard::start(
            message_and_format,
            self.generic_activity_with_arg(event_label, event_arg),
        )
        .with_jr_phase(event_label)
    }

    /// Start profiling a generic activity. Profiling continues until the
    /// TimingGuard returned from this call is dropped.
    #[inline(always)]
    pub fn generic_activity(&self, event_label: &'static str) -> TimingGuard<'_> {
        self.exec(EventFilter::GENERIC_ACTIVITIES, |profiler| {
            let event_label = profiler.get_or_alloc_cached_string(event_label);
            let event_id = EventId::from_label(event_label);
            TimingGuard::start(profiler, profiler.generic_activity_event_kind, event_id)
        })
    }

    /// Start profiling with some event filter for a given event. Profiling continues until the
    /// TimingGuard returned from this call is dropped.
    #[inline(always)]
    pub fn generic_activity_with_event_id(&self, event_id: EventId) -> TimingGuard<'_> {
        self.exec(EventFilter::GENERIC_ACTIVITIES, |profiler| {
            TimingGuard::start(profiler, profiler.generic_activity_event_kind, event_id)
        })
    }

    /// Start profiling a generic activity. Profiling continues until the
    /// TimingGuard returned from this call is dropped.
    #[inline(always)]
    pub fn generic_activity_with_arg<A>(
        &self,
        event_label: &'static str,
        event_arg: A,
    ) -> TimingGuard<'_>
    where
        A: Borrow<str> + Into<String>,
    {
        self.exec(EventFilter::GENERIC_ACTIVITIES, |profiler| {
            let builder = EventIdBuilder::new(&profiler.profiler);
            let event_label = profiler.get_or_alloc_cached_string(event_label);
            let event_id = if profiler.event_filter_mask.contains(EventFilter::FUNCTION_ARGS) {
                let event_arg = profiler.get_or_alloc_cached_string(event_arg);
                builder.from_label_and_arg(event_label, event_arg)
            } else {
                builder.from_label(event_label)
            };
            TimingGuard::start(profiler, profiler.generic_activity_event_kind, event_id)
        })
    }

    /// Start profiling a generic activity, allowing costly arguments to be recorded. Profiling
    /// continues until the `TimingGuard` returned from this call is dropped.
    ///
    /// If the arguments to a generic activity are cheap to create, use `generic_activity_with_arg`
    /// or `generic_activity_with_args` for their simpler API. However, if they are costly or
    /// require allocation in sufficiently hot contexts, then this allows for a closure to be called
    /// only when arguments were asked to be recorded via `-Z self-profile-events=args`.
    ///
    /// In this case, the closure will be passed a `&mut EventArgRecorder`, to help with recording
    /// one or many arguments within the generic activity being profiled, by calling its
    /// `record_arg` method for example.
    ///
    /// This `EventArgRecorder` may implement more specific traits from other rustc crates, e.g. for
    /// richer handling of rustc-specific argument types, while keeping this single entry-point API
    /// for recording arguments.
    ///
    /// Note: recording at least one argument is *required* for the self-profiler to create the
    /// `TimingGuard`. A panic will be triggered if that doesn't happen. This function exists
    /// explicitly to record arguments, so it fails loudly when there are none to record.
    ///
    #[inline(always)]
    pub fn generic_activity_with_arg_recorder<F>(
        &self,
        event_label: &'static str,
        mut f: F,
    ) -> TimingGuard<'_>
    where
        F: FnMut(&mut EventArgRecorder<'_>),
    {
        // Ensure this event will only be recorded when self-profiling is turned on.
        self.exec(EventFilter::GENERIC_ACTIVITIES, |profiler| {
            let builder = EventIdBuilder::new(&profiler.profiler);
            let event_label = profiler.get_or_alloc_cached_string(event_label);

            // Ensure the closure to create event arguments will only be called when argument
            // recording is turned on.
            let event_id = if profiler.event_filter_mask.contains(EventFilter::FUNCTION_ARGS) {
                // Set up the builder and call the user-provided closure to record potentially
                // costly event arguments.
                let mut recorder = EventArgRecorder { profiler, args: SmallVec::new() };
                f(&mut recorder);

                // It is expected that the closure will record at least one argument. If that
                // doesn't happen, it's a bug: we've been explicitly called in order to record
                // arguments, so we fail loudly when there are none to record.
                if recorder.args.is_empty() {
                    panic!(
                        "The closure passed to `generic_activity_with_arg_recorder` needs to \
                         record at least one argument"
                    );
                }

                builder.from_label_and_args(event_label, &recorder.args)
            } else {
                builder.from_label(event_label)
            };
            TimingGuard::start(profiler, profiler.generic_activity_event_kind, event_id)
        })
    }

    /// Record the size of an artifact that the compiler produces
    ///
    /// `artifact_kind` is the class of artifact (e.g., query_cache, object_file, etc.)
    /// `artifact_name` is an identifier to the specific artifact being stored (usually a filename)
    #[inline(always)]
    pub fn artifact_size<A>(&self, artifact_kind: &str, artifact_name: A, size: u64)
    where
        A: Borrow<str> + Into<String>,
    {
        drop(self.exec(EventFilter::ARTIFACT_SIZES, |profiler| {
            let builder = EventIdBuilder::new(&profiler.profiler);
            let event_label = profiler.get_or_alloc_cached_string(artifact_kind);
            let event_arg = profiler.get_or_alloc_cached_string(artifact_name);
            let event_id = builder.from_label_and_arg(event_label, event_arg);
            let thread_id = get_thread_id();

            profiler.profiler.record_integer_event(
                profiler.artifact_size_event_kind,
                event_id,
                thread_id,
                size,
            );

            TimingGuard::none()
        }))
    }

    #[inline(always)]
    pub fn generic_activity_with_args(
        &self,
        event_label: &'static str,
        event_args: &[String],
    ) -> TimingGuard<'_> {
        self.exec(EventFilter::GENERIC_ACTIVITIES, |profiler| {
            let builder = EventIdBuilder::new(&profiler.profiler);
            let event_label = profiler.get_or_alloc_cached_string(event_label);
            let event_id = if profiler.event_filter_mask.contains(EventFilter::FUNCTION_ARGS) {
                let event_args: Vec<_> = event_args
                    .iter()
                    .map(|s| profiler.get_or_alloc_cached_string(&s[..]))
                    .collect();
                builder.from_label_and_args(event_label, &event_args)
            } else {
                builder.from_label(event_label)
            };
            TimingGuard::start(profiler, profiler.generic_activity_event_kind, event_id)
        })
    }

    /// Start profiling a query provider. Profiling continues until the
    /// TimingGuard returned from this call is dropped.
    #[inline(always)]
    pub fn query_provider(&self) -> TimingGuard<'_> {
        self.exec(EventFilter::QUERY_PROVIDERS, |profiler| {
            TimingGuard::start(profiler, profiler.query_event_kind, EventId::INVALID)
        })
    }

    /// Record a query in-memory cache hit.
    #[inline(always)]
    pub fn query_cache_hit(&self, query_invocation_id: QueryInvocationId) {
        #[inline(never)]
        #[cold]
        fn cold_call(profiler_ref: &SelfProfilerRef, query_invocation_id: QueryInvocationId) {
            if profiler_ref.event_filter_mask.contains(EventFilter::QUERY_CACHE_HIT_COUNTS) {
                profiler_ref
                    .profiler
                    .as_ref()
                    .unwrap()
                    .increment_query_cache_hit_counters(QueryInvocationId(query_invocation_id.0));
            }
            if profiler_ref.event_filter_mask.contains(EventFilter::QUERY_CACHE_HITS) {
                hint::cold_path();
                profiler_ref.instant_query_event(
                    |profiler| profiler.query_cache_hit_event_kind,
                    query_invocation_id,
                );
            }
        }

        // We check both kinds of query cache hit events at once, to reduce overhead in the
        // common case (with self-profile disabled).
        if self.event_filter_mask.intersects(EventFilter::QUERY_CACHE_HIT_COMBINED) {
            hint::cold_path();
            cold_call(self, query_invocation_id);
        }
    }

    /// Start profiling a query being blocked on a concurrent execution.
    /// Profiling continues until the TimingGuard returned from this call is
    /// dropped.
    #[inline(always)]
    pub fn query_blocked(&self) -> TimingGuard<'_> {
        self.exec(EventFilter::QUERY_BLOCKED, |profiler| {
            TimingGuard::start(profiler, profiler.query_blocked_event_kind, EventId::INVALID)
        })
    }

    /// Start profiling how long it takes to load a query result from the
    /// incremental compilation on-disk cache. Profiling continues until the
    /// TimingGuard returned from this call is dropped.
    #[inline(always)]
    pub fn incr_cache_loading(&self) -> TimingGuard<'_> {
        self.exec(EventFilter::INCR_CACHE_LOADS, |profiler| {
            TimingGuard::start(
                profiler,
                profiler.incremental_load_result_event_kind,
                EventId::INVALID,
            )
        })
    }

    /// Start profiling how long it takes to hash query results for incremental compilation.
    /// Profiling continues until the TimingGuard returned from this call is dropped.
    #[inline(always)]
    pub fn incr_result_hashing(&self) -> TimingGuard<'_> {
        self.exec(EventFilter::INCR_RESULT_HASHING, |profiler| {
            TimingGuard::start(
                profiler,
                profiler.incremental_result_hashing_event_kind,
                EventId::INVALID,
            )
        })
    }

    #[inline(always)]
    fn instant_query_event(
        &self,
        event_kind: fn(&SelfProfiler) -> StringId,
        query_invocation_id: QueryInvocationId,
    ) {
        let event_id = StringId::new_virtual(query_invocation_id.0);
        let thread_id = get_thread_id();
        let profiler = self.profiler.as_ref().unwrap();
        profiler.profiler.record_instant_event(
            event_kind(profiler),
            EventId::from_virtual(event_id),
            thread_id,
        );
    }

    pub fn with_profiler(&self, f: impl FnOnce(&SelfProfiler)) {
        if let Some(profiler) = &self.profiler {
            f(profiler)
        }
    }

    /// Gets a `StringId` for the given string. This method makes sure that
    /// any strings going through it will only be allocated once in the
    /// profiling data.
    /// Returns `None` if the self-profiling is not enabled.
    pub fn get_or_alloc_cached_string(&self, s: &str) -> Option<StringId> {
        self.profiler.as_ref().map(|p| p.get_or_alloc_cached_string(s))
    }

    /// Store query cache hits to the self-profile log.
    /// Should be called once at the end of the compilation session.
    ///
    /// The cache hits are stored per **query invocation**, not **per query kind/type**.
    /// `analyzeme` can later deduplicate individual query labels from the QueryInvocationId event
    /// IDs.
    pub fn store_query_cache_hits(&self) {
        if self.event_filter_mask.contains(EventFilter::QUERY_CACHE_HIT_COUNTS) {
            let profiler = self.profiler.as_ref().unwrap();
            let query_hits = profiler.query_hits.read();
            let builder = EventIdBuilder::new(&profiler.profiler);
            let thread_id = get_thread_id();
            for (query_invocation, hit_count) in query_hits.iter().enumerate() {
                let hit_count = hit_count.load(Ordering::Relaxed);
                // No need to record empty cache hit counts
                if hit_count > 0 {
                    let event_id =
                        builder.from_label(StringId::new_virtual(query_invocation as u64));
                    profiler.profiler.record_integer_event(
                        profiler.query_cache_hit_count_event_kind,
                        event_id,
                        thread_id,
                        hit_count,
                    );
                }
            }
        }
    }

    #[inline]
    pub fn enabled(&self) -> bool {
        self.profiler.is_some()
    }

    #[inline]
    pub fn llvm_recording_enabled(&self) -> bool {
        self.event_filter_mask.contains(EventFilter::LLVM)
    }
    #[inline]
    pub fn get_self_profiler(&self) -> Option<Arc<SelfProfiler>> {
        self.profiler.clone()
    }

    /// Is expensive recording of query keys and/or function arguments enabled?
    pub fn is_args_recording_enabled(&self) -> bool {
        self.enabled() && self.event_filter_mask.intersects(EventFilter::ARGS)
    }
}

/// A helper for recording costly arguments to self-profiling events. Used with
/// `SelfProfilerRef::generic_activity_with_arg_recorder`.
pub struct EventArgRecorder<'p> {
    /// The `SelfProfiler` used to intern the event arguments that users will ask to record.
    profiler: &'p SelfProfiler,

    /// The interned event arguments to be recorded in the generic activity event.
    ///
    /// The most common case, when actually recording event arguments, is to have one argument. Then
    /// followed by recording two, in a couple places.
    args: SmallVec<[StringId; 2]>,
}

impl EventArgRecorder<'_> {
    /// Records a single argument within the current generic activity being profiled.
    ///
    /// Note: when self-profiling with costly event arguments, at least one argument
    /// needs to be recorded. A panic will be triggered if that doesn't happen.
    pub fn record_arg<A>(&mut self, event_arg: A)
    where
        A: Borrow<str> + Into<String>,
    {
        let event_arg = self.profiler.get_or_alloc_cached_string(event_arg);
        self.args.push(event_arg);
    }
}

pub struct SelfProfiler {
    profiler: Profiler,
    event_filter_mask: EventFilter,

    string_cache: RwLock<FxHashMap<String, StringId>>,

    /// Recording individual query cache hits as "instant" measureme events
    /// is incredibly expensive. Instead of doing that, we simply aggregate
    /// cache hit *counts* per query invocation, and then store the final count
    /// of cache hits per invocation at the end of the compilation session.
    ///
    /// With this approach, we don't know the individual thread IDs and timestamps
    /// of cache hits, but it has very little overhead on top of `-Zself-profile`.
    /// Recording the cache hits as individual events made compilation 3-5x slower.
    ///
    /// Query invocation IDs should be monotonic integers, so we can store them in a vec,
    /// rather than using a hashmap.
    query_hits: RwLock<Vec<AtomicU64>>,

    query_event_kind: StringId,
    generic_activity_event_kind: StringId,
    incremental_load_result_event_kind: StringId,
    incremental_result_hashing_event_kind: StringId,
    query_blocked_event_kind: StringId,
    query_cache_hit_event_kind: StringId,
    artifact_size_event_kind: StringId,
    /// Total cache hits per query invocation
    query_cache_hit_count_event_kind: StringId,
}

impl SelfProfiler {
    pub fn new(
        output_directory: &Path,
        crate_name: Option<&str>,
        event_filters: Option<&[String]>,
        counter_name: &str,
    ) -> Result<SelfProfiler, Box<dyn Error + Send + Sync>> {
        fs::create_dir_all(output_directory)?;

        let crate_name = crate_name.unwrap_or("unknown-crate");
        // HACK(eddyb) we need to pad the PID, strange as it may seem, as its
        // length can behave as a source of entropy for heap addresses, when
        // ASLR is disabled and the heap is otherwise deterministic.
        let pid: u32 = process::id();
        let filename = format!("{crate_name}-{pid:07}.rustc_profile");
        let path = output_directory.join(filename);
        let profiler =
            Profiler::with_counter(&path, measureme::counters::Counter::by_name(counter_name)?)?;

        let query_event_kind = profiler.alloc_string("Query");
        let generic_activity_event_kind = profiler.alloc_string("GenericActivity");
        let incremental_load_result_event_kind = profiler.alloc_string("IncrementalLoadResult");
        let incremental_result_hashing_event_kind =
            profiler.alloc_string("IncrementalResultHashing");
        let query_blocked_event_kind = profiler.alloc_string("QueryBlocked");
        let query_cache_hit_event_kind = profiler.alloc_string("QueryCacheHit");
        let artifact_size_event_kind = profiler.alloc_string("ArtifactSize");
        let query_cache_hit_count_event_kind = profiler.alloc_string("QueryCacheHitCount");

        let mut event_filter_mask = EventFilter::empty();

        if let Some(event_filters) = event_filters {
            let mut unknown_events = vec![];
            for item in event_filters {
                if let Some(&(_, mask)) =
                    EVENT_FILTERS_BY_NAME.iter().find(|&(name, _)| name == item)
                {
                    event_filter_mask |= mask;
                } else {
                    unknown_events.push(item.clone());
                }
            }

            // Warn about any unknown event names
            if !unknown_events.is_empty() {
                unknown_events.sort();
                unknown_events.dedup();

                warn!(
                    "Unknown self-profiler events specified: {}. Available options are: {}.",
                    unknown_events.join(", "),
                    EVENT_FILTERS_BY_NAME
                        .iter()
                        .map(|&(name, _)| name.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        } else {
            event_filter_mask = EventFilter::DEFAULT;
        }

        Ok(SelfProfiler {
            profiler,
            event_filter_mask,
            string_cache: RwLock::new(FxHashMap::default()),
            query_event_kind,
            generic_activity_event_kind,
            incremental_load_result_event_kind,
            incremental_result_hashing_event_kind,
            query_blocked_event_kind,
            query_cache_hit_event_kind,
            artifact_size_event_kind,
            query_cache_hit_count_event_kind,
            query_hits: Default::default(),
        })
    }

    /// Allocates a new string in the profiling data. Does not do any caching
    /// or deduplication.
    pub fn alloc_string<STR: SerializableString + ?Sized>(&self, s: &STR) -> StringId {
        self.profiler.alloc_string(s)
    }

    /// Store a cache hit of a query invocation
    pub fn increment_query_cache_hit_counters(&self, id: QueryInvocationId) {
        // Fast path: assume that the query was already encountered before, and just record
        // a cache hit.
        let mut guard = self.query_hits.upgradable_read();
        let query_hits = &guard;
        let index = id.0 as usize;
        if index < query_hits.len() {
            // We only want to increment the count, no other synchronization is required
            query_hits[index].fetch_add(1, Ordering::Relaxed);
        } else {
            // If not, we need to extend the query hit map to the highest observed ID
            guard.with_upgraded(|vec| {
                vec.resize_with(index + 1, || AtomicU64::new(0));
                vec[index] = AtomicU64::from(1);
            });
        }
    }

    /// Gets a `StringId` for the given string. This method makes sure that
    /// any strings going through it will only be allocated once in the
    /// profiling data.
    pub fn get_or_alloc_cached_string<A>(&self, s: A) -> StringId
    where
        A: Borrow<str> + Into<String>,
    {
        // Only acquire a read-lock first since we assume that the string is
        // already present in the common case.
        {
            let string_cache = self.string_cache.read();

            if let Some(&id) = string_cache.get(s.borrow()) {
                return id;
            }
        }

        let mut string_cache = self.string_cache.write();
        // Check if the string has already been added in the small time window
        // between dropping the read lock and acquiring the write lock.
        match string_cache.entry(s.into()) {
            Entry::Occupied(e) => *e.get(),
            Entry::Vacant(e) => {
                let string_id = self.profiler.alloc_string(&e.key()[..]);
                *e.insert(string_id)
            }
        }
    }

    pub fn map_query_invocation_id_to_string(&self, from: QueryInvocationId, to: StringId) {
        let from = StringId::new_virtual(from.0);
        self.profiler.map_virtual_to_concrete_string(from, to);
    }

    pub fn bulk_map_query_invocation_id_to_single_string<I>(&self, from: I, to: StringId)
    where
        I: Iterator<Item = QueryInvocationId> + ExactSizeIterator,
    {
        let from = from.map(|qid| StringId::new_virtual(qid.0));
        self.profiler.bulk_map_virtual_to_single_concrete_string(from, to);
    }

    pub fn query_key_recording_enabled(&self) -> bool {
        self.event_filter_mask.contains(EventFilter::QUERY_KEYS)
    }

    pub fn event_id_builder(&self) -> EventIdBuilder<'_> {
        EventIdBuilder::new(&self.profiler)
    }
}

#[must_use]
pub struct TimingGuard<'a>(Option<measureme::TimingGuard<'a>>);

impl<'a> TimingGuard<'a> {
    #[inline]
    pub fn start(
        profiler: &'a SelfProfiler,
        event_kind: StringId,
        event_id: EventId,
    ) -> TimingGuard<'a> {
        let thread_id = get_thread_id();
        let raw_profiler = &profiler.profiler;
        let timing_guard =
            raw_profiler.start_recording_interval_event(event_kind, event_id, thread_id);
        TimingGuard(Some(timing_guard))
    }

    #[inline]
    pub fn finish_with_query_invocation_id(self, query_invocation_id: QueryInvocationId) {
        if let Some(guard) = self.0 {
            outline(|| {
                let event_id = StringId::new_virtual(query_invocation_id.0);
                let event_id = EventId::from_virtual(event_id);
                guard.finish_with_override_event_id(event_id);
            });
        }
    }

    #[inline]
    pub fn none() -> TimingGuard<'a> {
        TimingGuard(None)
    }

    #[inline(always)]
    pub fn run<R>(self, f: impl FnOnce() -> R) -> R {
        let _timer = self;
        f()
    }
}

struct VerboseInfo {
    start_time: Instant,
    start_rss: Option<usize>,
    message: String,
    format: TimePassesFormat,
}

#[must_use]
pub struct VerboseTimingGuard<'a> {
    info: Option<VerboseInfo>,
    _guard: TimingGuard<'a>,
    _jr_phase: JrPhaseGuard,
}

impl<'a> VerboseTimingGuard<'a> {
    pub fn start(
        message_and_format: Option<(String, TimePassesFormat)>,
        _guard: TimingGuard<'a>,
    ) -> Self {
        VerboseTimingGuard {
            _guard,
            _jr_phase: JrPhaseGuard(None),
            info: message_and_format.map(|(message, format)| VerboseInfo {
                start_time: Instant::now(),
                start_rss: get_resident_set_size(),
                message,
                format,
            }),
        }
    }

    #[inline(always)]
    pub fn run<R>(self, f: impl FnOnce() -> R) -> R {
        let _timer = self;
        f()
    }

    /// justrust: also account this activity in the phase ledger.
    #[inline]
    fn with_jr_phase(mut self, label: &'static str) -> Self {
        self._jr_phase = jr_phase(label);
        self
    }
}

/// justrust: the phase ledger. With `JUSTRUST_RUSTC_PHASES_FILE=<path>` in
/// the environment the compiler sums the wall time of every verbose activity
/// (what `-Ztime-passes` prints) per label in memory and writes one small
/// file when it exits: `label\tseconds\tcount` per line.
///
/// This is what lets the build recorder show a per-phase split for every
/// unit by default. `-Ztime-passes` is not usable for that: it prints one
/// stderr line and reads `/proc/self/statm` twice per event, and `drop_ast`
/// alone fires once per item (12,356 of 12,404 events on a 92k-line crate,
/// +5% wall). The ledger costs two clock reads and one uncontended lock per
/// event, does no I/O until exit, and is not a compiler option, so it cannot
/// change an incremental or artifact cache key.
///
/// Activities that run on worker threads (`jr_phase` call sites in codegen)
/// are summed over threads, so their total can exceed wall time.
struct JrPhaseLedger {
    path: std::path::PathBuf,
    /// When this compiler session started (`jr_phases_begin`).
    start: Instant,
    /// Set once the file is written: an early reply writes it before the
    /// process ends, and the reader may already have consumed it by then.
    written: std::sync::atomic::AtomicBool,
    phases: parking_lot::Mutex<FxHashMap<&'static str, (Duration, u64)>>,
    /// The reuse ledger: per (outcome, query) the exclusive provider or load
    /// time and count. See `jr_reuse_finish`.
    reuse: parking_lot::Mutex<FxHashMap<(&'static str, &'static str), (Duration, u64)>>,
    /// Why nodes could not be marked green: per (node kind, first red
    /// dependency kind, how the dependency was found red) a count. See
    /// `jr_red_dep`.
    red: parking_lot::Mutex<FxHashMap<(&'static str, &'static str, &'static str), u64>>,
}

thread_local! {
    /// Exclusive time of completed reuse-ledger scopes on this thread, so a
    /// parent query does not also own the time of the queries it called.
    static JR_REUSE_COMPLETED: std::cell::Cell<u128> = const { std::cell::Cell::new(0) };
}

/// Whether the reuse ledger is on (the phase ledger is). One cached load.
#[inline]
pub fn jr_reuse_enabled() -> bool {
    jr_phase_ledger().is_some()
}

/// A started reuse-ledger scope: `Instant` plus the completed nested time.
#[derive(Clone, Copy)]
pub struct JrReuseStart(Instant, u128);

/// Start a reuse-ledger scope around a query provider invocation or load.
/// `None` (free) when the ledger is off.
#[inline]
pub fn jr_reuse_start() -> Option<JrReuseStart> {
    jr_reuse_enabled().then(|| JrReuseStart(Instant::now(), JR_REUSE_COMPLETED.with(|c| c.get())))
}

/// Finish a reuse-ledger scope as `outcome` for `query`. Outcomes are what
/// incremental compilation did with this node:
///
/// * `loaded`: green, value loaded from the incremental cache (reused)
/// * `green_recompute`: green, but no cached value so the provider ran again
/// * `new`: no node in the previous session, provider ran
/// * `same`: inputs changed, provider re-ran, produced the identical value
///   (work a finer-grained dependency would have avoided)
/// * `changed`: re-ran and produced a different value (necessary work)
/// * `nohash`: re-ran, result is never hashed so reuse is impossible
/// * `eval_always`: re-runs every session by design
///
/// The time is exclusive of nested reuse-ledger scopes on the same thread.
pub fn jr_reuse_finish(start: JrReuseStart, query: &'static str, outcome: &'static str) {
    let Some(ledger) = jr_phase_ledger() else { return };
    let elapsed = start.0.elapsed().as_nanos();
    let exclusive = JR_REUSE_COMPLETED.with(|c| {
        let now = c.get();
        let exclusive = elapsed.saturating_sub(now.saturating_sub(start.1));
        c.set(now.saturating_add(exclusive));
        exclusive
    });
    let dur = Duration::from_nanos(exclusive.min(u64::MAX as u128) as u64);
    let mut reuse = ledger.reuse.lock();
    let entry = reuse.entry((outcome, query)).or_insert((Duration::ZERO, 0));
    entry.0 += dur;
    entry.1 += 1;
}

/// A node of kind `node` could not be marked green because its dependency of
/// kind `dep` is red. Only the first red dependency is seen: marking stops
/// there. `how` is `red_dep` (the dependency re-ran to a different value or
/// is never hashed) or `red_dep_unforced` (the dependency could not be
/// forced from its dep node, so nothing is known about its value). Written
/// to the reuse ledger as `how\t<node<-dep>\t0\tcount`.
pub fn jr_red_dep(node: &'static str, dep: &'static str, how: &'static str) {
    if let Some(ledger) = jr_phase_ledger() {
        *ledger.red.lock().entry((node, dep, how)).or_insert(0) += 1;
    }
}

/// Count `n` units of `outcome` for a non-query item kind (codegen units).
pub fn jr_reuse_count(kind: &'static str, outcome: &'static str, dur: Duration) {
    jr_reuse_add(kind, outcome, dur, 1);
}

/// Add `count` items and `dur` to (`outcome`, `kind`) in the reuse ledger.
pub fn jr_reuse_add(kind: &'static str, outcome: &'static str, dur: Duration, count: u64) {
    if let Some(ledger) = jr_phase_ledger() {
        let mut reuse = ledger.reuse.lock();
        let entry = reuse.entry((outcome, kind)).or_insert((Duration::ZERO, 0));
        entry.0 += dur;
        entry.1 += count;
    }
}

/// CPU time this thread has used so far. `None` where unsupported. Used to
/// charge codegen and LLVM work to codegen units in CPU, not wall, time.
#[inline]
pub fn jr_thread_cpu() -> Option<Duration> {
    #[cfg(unix)]
    {
        let mut ts = std::mem::MaybeUninit::<libc::timespec>::uninit();
        // SAFETY: plain syscall writing into a local timespec.
        if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, ts.as_mut_ptr()) } != 0 {
            return None;
        }
        let ts = unsafe { ts.assume_init() };
        Some(Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32))
    }
    #[cfg(not(unix))]
    None
}

/// CPU time of waited-for children (the linker). `None` where unsupported.
pub fn jr_children_cpu() -> Option<Duration> {
    #[cfg(unix)]
    {
        let mut ru = std::mem::MaybeUninit::<libc::rusage>::uninit();
        // SAFETY: plain syscall writing into a local rusage.
        if unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, ru.as_mut_ptr()) } != 0 {
            return None;
        }
        let ru = unsafe { ru.assume_init() };
        let tv = |t: libc::timeval| Duration::new(t.tv_sec as u64, t.tv_usec as u32 * 1000);
        Some(tv(ru.ru_utime) + tv(ru.ru_stime))
    }
    #[cfg(not(unix))]
    None
}

/// Charges this thread's CPU time until dropped to (`outcome`, `kind`).
#[must_use]
pub struct JrCpuGuard(Option<(&'static str, &'static str, Duration)>);

impl Drop for JrCpuGuard {
    fn drop(&mut self) {
        if let Some((kind, outcome, start)) = self.0
            && let Some(now) = jr_thread_cpu()
        {
            jr_reuse_count(kind, outcome, now.saturating_sub(start));
        }
    }
}

/// Start charging this thread's CPU to (`outcome`, `kind`). Free when off.
#[inline]
pub fn jr_cpu(kind: &'static str, outcome: &'static str) -> JrCpuGuard {
    JrCpuGuard(if jr_reuse_enabled() { jr_thread_cpu().map(|t| (kind, outcome, t)) } else { None })
}

/// Per-crate cost history, kept in the incremental session directory so
/// reuse can be credited with what the reused work measurably cost in this
/// crate's own earlier sessions, not a guess. `query_kind -> (secs, count)`
/// of provider executions summed over sessions, and `cgu name -> cpu secs`
/// of the last time each codegen unit was actually compiled.
#[derive(Default)]
pub struct JrCostHistory {
    pub queries: FxHashMap<String, (f64, u64)>,
    pub cgus: FxHashMap<String, f64>,
}

/// Sidecar file name in the incremental session directory.
pub const JR_COSTS_FILE: &str = "jr-costs.tsv";

static JR_PRIOR_COSTS: std::sync::OnceLock<JrCostHistory> = std::sync::OnceLock::new();

impl JrCostHistory {
    fn parse(text: &str) -> Self {
        let mut h = JrCostHistory::default();
        for l in text.lines() {
            let mut f = l.split('\t');
            match (f.next(), f.next(), f.next(), f.next()) {
                (Some("q"), Some(q), Some(s), Some(n)) => {
                    if let (Ok(s), Ok(n)) = (s.parse::<f64>(), n.parse::<u64>())
                        && s.is_finite()
                    {
                        h.queries.insert(q.to_owned(), (s, n));
                    }
                }
                (Some("c"), Some(c), Some(s), None) => {
                    if let Ok(s) = s.parse::<f64>()
                        && s.is_finite()
                    {
                        h.cgus.insert(c.to_owned(), s);
                    }
                }
                _ => {}
            }
        }
        h
    }
}

/// Read the previous session's cost history (once). Best effort.
pub fn jr_costs_load(old_session_dir: Option<&Path>) {
    if !jr_reuse_enabled() {
        return;
    }
    let h = old_session_dir
        .and_then(|d| fs::read_to_string(d.join(JR_COSTS_FILE)).ok())
        .map(|t| JrCostHistory::parse(&t))
        .unwrap_or_default();
    let _ = JR_PRIOR_COSTS.set(h);
}

/// Mean measured provider seconds of `query` in this crate's history.
fn jr_prior_query_mean(query: &str) -> Option<f64> {
    let (s, n) = *JR_PRIOR_COSTS.get()?.queries.get(query)?;
    (n > 0).then(|| s / n as f64)
}

/// A codegen unit was reused: credit what it cost when last compiled.
pub fn jr_cgu_reused(cgu: &str) {
    if !jr_reuse_enabled() {
        return;
    }
    match JR_PRIOR_COSTS.get().and_then(|h| h.cgus.get(cgu)) {
        Some(&s) => {
            jr_reuse_count("<cgu_avoided>", "measured", Duration::from_secs_f64(s));
            JR_CGU_COSTS.lock().insert(cgu.to_owned(), s);
        }
        None => jr_reuse_count("<cgu_avoided>", "unmeasured", Duration::ZERO),
    }
}

static JR_CGU_COSTS: std::sync::LazyLock<parking_lot::Mutex<FxHashMap<String, f64>>> =
    std::sync::LazyLock::new(Default::default);
static JR_CGU_OUTCOME: std::sync::LazyLock<parking_lot::Mutex<FxHashMap<String, &'static str>>> =
    std::sync::LazyLock::new(Default::default);

/// Note what incremental decided for codegen unit `cgu` (`new`, `changed`).
pub fn jr_cgu_outcome(cgu: &str, outcome: &'static str) {
    if jr_reuse_enabled() {
        JR_CGU_OUTCOME.lock().insert(cgu.to_owned(), outcome);
    }
}

/// Add CPU spent compiling codegen unit `cgu` this session (frontend
/// codegen and LLVM), carried into the cost history and charged to the
/// `<cgu_cpu>` row of the unit's outcome.
pub fn jr_cgu_cost(cgu: &str, cpu: Duration) {
    if jr_reuse_enabled() {
        let outcome = JR_CGU_OUTCOME.lock().get(cgu).copied().unwrap_or("untracked");
        jr_reuse_add("<cgu_cpu>", outcome, cpu, 0);
        *JR_CGU_COSTS.lock().entry(cgu.to_owned()).or_insert(0.0) += cpu.as_secs_f64();
    }
}

/// Charges this thread's CPU until dropped to codegen unit `cgu`, less the
/// time of query scopes that completed inside it on this thread (those are
/// already in the query rows), so the ledger never counts work twice.
#[must_use]
pub struct JrCguCpuGuard(Option<(String, Duration, u128)>);

impl Drop for JrCguCpuGuard {
    fn drop(&mut self) {
        if let Some((cgu, start, nested_at)) = self.0.take()
            && let Some(now) = jr_thread_cpu()
        {
            let nested = JR_REUSE_COMPLETED.with(|c| c.get()).saturating_sub(nested_at);
            let nested = Duration::from_nanos(nested.min(u64::MAX as u128) as u64);
            jr_cgu_cost(&cgu, now.saturating_sub(start).saturating_sub(nested));
        }
    }
}

/// Start charging this thread's CPU to codegen unit `cgu`. Free when off.
#[inline]
pub fn jr_cgu_cpu(cgu: &str) -> JrCguCpuGuard {
    JrCguCpuGuard(if jr_reuse_enabled() {
        jr_thread_cpu().map(|t| (cgu.to_owned(), t, JR_REUSE_COMPLETED.with(|c| c.get())))
    } else {
        None
    })
}

/// Record where this session's incremental state came from: `<session>`
/// rows `warm`, `cold_no_prior`, `cold_out_of_date`, `cold_io_error`, plus
/// `<sibling_state>` `present` when the incremental dir holds state for
/// the same crate name under another crate id (another `-C metadata`:
/// features, profile or target), i.e. a cold start next to reusable work.
pub fn jr_session_source(outcome: &'static str, siblings: u64) {
    jr_reuse_count("<session>", outcome, Duration::ZERO);
    if siblings > 0 {
        jr_reuse_add("<sibling_state>", "present", Duration::ZERO, siblings);
    }
}

/// This session had no state of its own lineage and started from the newest
/// state of the same crate id in another one: row `<session_seed>` `sibling`.
/// The `<session>` row says whether that state loaded.
pub fn jr_session_seeded() {
    jr_reuse_count("<session_seed>", "sibling", Duration::ZERO);
}

/// The linker ran for `cpu` (children CPU while it ran).
pub fn jr_link_cpu(cpu: Duration) {
    jr_reuse_count("<link>", "linker", cpu);
}

/// Write this session's cost history into `new_session_dir`: previous query
/// totals plus this session's executions, and the cost of every codegen
/// unit compiled or carried forward. Best effort.
pub fn jr_costs_save(new_session_dir: &Path) {
    use std::fmt::Write;
    let Some(ledger) = jr_phase_ledger() else { return };
    let mut q: FxHashMap<String, (f64, u64)> =
        JR_PRIOR_COSTS.get().map(|h| h.queries.clone()).unwrap_or_default();
    for ((outcome, query), (dur, count)) in ledger.reuse.lock().iter() {
        if query.starts_with('<') {
            continue;
        }
        if matches!(
            *outcome,
            "new" | "changed" | "same" | "green_recompute" | "nohash" | "eval_always"
                | "eval_always_same" | "eval_always_changed"
        ) {
            let e = q.entry((*query).to_owned()).or_insert((0.0, 0));
            e.0 += dur.as_secs_f64();
            e.1 += count;
        }
    }
    let mut out = String::new();
    let mut qs: Vec<_> = q.into_iter().collect();
    qs.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, (s, n)) in qs {
        // Keep the history bounded and recent: halve once it is large.
        let (s, n) = if n > 1 << 24 { (s / 2.0, n / 2) } else { (s, n) };
        let _ = writeln!(out, "q\t{name}\t{s:.9}\t{n}");
    }
    let cgus = JR_CGU_COSTS.lock();
    let mut cs: Vec<_> = cgus.iter().collect();
    cs.sort_by(|a, b| a.0.cmp(b.0));
    for (name, s) in cs {
        let _ = writeln!(out, "c\t{name}\t{s:.6}");
    }
    let path = new_session_dir.join(JR_COSTS_FILE);
    let tmp = path.with_extension("tsv-tmp");
    if fs::write(&tmp, out).is_ok() {
        let _ = fs::rename(&tmp, &path);
    }
}

/// Credit loaded query results with this crate's measured mean cost.
fn jr_credit_loads(reuse: &FxHashMap<(&'static str, &'static str), (Duration, u64)>) -> Vec<(&'static str, Duration, u64, bool)> {
    let mut v = Vec::new();
    for ((outcome, query), (_, count)) in reuse.iter() {
        if *outcome == "loaded" && !query.starts_with('<') {
            match jr_prior_query_mean(query) {
                Some(m) => v.push((*query, Duration::from_secs_f64(m * *count as f64), *count, true)),
                None => v.push((*query, Duration::ZERO, *count, false)),
            }
        }
    }
    v
}

static JR_PHASES_START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// Tell the phase ledger when the compiler session started, so a ledger
/// written before the end (`jr_phases_write_now`) has the right total.
pub fn jr_phases_begin(start: Instant) {
    let _ = JR_PHASES_START.set(start);
}

fn jr_phase_ledger() -> Option<&'static JrPhaseLedger> {
    static LEDGER: std::sync::OnceLock<Option<JrPhaseLedger>> = std::sync::OnceLock::new();
    LEDGER
        .get_or_init(|| {
            let path = std::env::var_os("JUSTRUST_RUSTC_PHASES_FILE")?;
            (!path.is_empty()).then(|| JrPhaseLedger {
                path: path.into(),
                start: *JR_PHASES_START.get_or_init(Instant::now),
                written: std::sync::atomic::AtomicBool::new(false),
                phases: parking_lot::Mutex::new(FxHashMap::default()),
                reuse: parking_lot::Mutex::new(FxHashMap::default()),
                red: parking_lot::Mutex::new(FxHashMap::default()),
            })
        })
        .as_ref()
}

/// Accounts the time until it is dropped to `label` in the phase ledger.
#[must_use]
pub struct JrPhaseGuard(Option<(&'static str, Instant)>);

impl Drop for JrPhaseGuard {
    #[inline]
    fn drop(&mut self) {
        if let Some((label, start)) = self.0 {
            jr_phase_add(label, start.elapsed());
        }
    }
}

/// Start timing `label` for the phase ledger. Free when the ledger is off.
#[inline]
pub fn jr_phase(label: &'static str) -> JrPhaseGuard {
    JrPhaseGuard(jr_phase_ledger().map(|_| (label, Instant::now())))
}

/// Add an already measured duration to `label` in the phase ledger.
pub fn jr_phase_add(label: &'static str, dur: Duration) {
    if let Some(ledger) = jr_phase_ledger() {
        let mut phases = ledger.phases.lock();
        let entry = phases.entry(label).or_insert((Duration::ZERO, 0));
        entry.0 += dur;
        entry.1 += 1;
    }
}

/// Write the phase ledger, with `total` as the whole compiler session.
/// Best effort: profiling never fails a compilation. Does nothing when the
/// ledger was already written (`jr_phases_write_now`).
pub fn jr_phases_write(total: Duration) {
    use std::fmt::Write;
    let Some(ledger) = jr_phase_ledger() else { return };
    if ledger.written.swap(true, Ordering::Relaxed) {
        return;
    }
    let mut out = String::new();
    {
        let phases = ledger.phases.lock();
        let mut rows: Vec<_> = phases.iter().map(|(label, v)| (*label, *v)).collect();
        rows.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(b.0)));
        for (label, (dur, count)) in rows {
            let _ = writeln!(out, "{label}\t{:.6}\t{count}", dur.as_secs_f64());
        }
    }
    // The reuse ledger goes beside it (`<path>.reuse`, written first), so
    // readers that predate it never mistake its rows for phases:
    // `outcome\tquery\tseconds\tcount` per line.
    let mut reuse_out = String::new();
    {
        let mut reuse = ledger.reuse.lock();
        // Loads are credited with their measured history once, at write
        // time (an early-reply rewrite replaces the previous credit).
        reuse.retain(|(o, _), _| *o != "loaded_avoided" && *o != "loaded_unmeasured");
        for (query, dur, count, measured) in jr_credit_loads(&reuse) {
            let o = if measured { "loaded_avoided" } else { "loaded_unmeasured" };
            let e = reuse.entry((o, query)).or_insert((Duration::ZERO, 0));
            e.0 += dur;
            e.1 += count;
        }
        let mut rows: Vec<_> = reuse.iter().map(|(k, v)| (*k, *v)).collect();
        rows.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(&b.0)));
        for ((outcome, query), (dur, count)) in rows {
            let _ = writeln!(reuse_out, "{outcome}\t{query}\t{:.6}\t{count}", dur.as_secs_f64());
        }
    }
    {
        // `<...>` marks an evidence row: readers never sum it as query time.
        let red = ledger.red.lock();
        let mut rows: Vec<_> = red.iter().map(|(k, v)| (*k, *v)).collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        for ((node, dep, how), count) in rows {
            let _ = writeln!(reuse_out, "{how}\t<{node}<-{dep}>\t0.000000\t{count}");
        }
    }
    if !reuse_out.is_empty() {
        let mut name = ledger.path.clone().into_os_string();
        name.push(".reuse");
        let path = std::path::PathBuf::from(name);
        let tmp = path.with_extension("reuse-tmp");
        if fs::write(&tmp, reuse_out).is_ok() {
            let _ = fs::rename(&tmp, &path);
        }
    }
    let _ = writeln!(out, "total\t{:.6}\t1", total.as_secs_f64());
    // The reader only looks after this process exited, and a rename keeps a
    // killed compiler from leaving half a file.
    let tmp = ledger.path.with_extension("tmp");
    if fs::write(&tmp, out).is_ok() {
        let _ = fs::rename(&tmp, &ledger.path);
    }
}

/// Write the phase ledger now, with the time since the session started as
/// its total. For the early reply: the caller stops waiting here and reads
/// the file at once, so this is the time it waited for. Work after this
/// point (the background incremental persist) is deliberately left out, and
/// the file is not written again at exit.
pub fn jr_phases_write_now() {
    if let Some(ledger) = jr_phase_ledger() {
        jr_phases_write(ledger.start.elapsed());
    }
}

/// Undo `jr_phases_write_now`: the ledger is written again at exit.
pub fn jr_phases_rearm() {
    if let Some(ledger) = jr_phase_ledger() {
        ledger.written.store(false, Ordering::Relaxed);
    }
}

impl Drop for VerboseTimingGuard<'_> {
    fn drop(&mut self) {
        if let Some(info) = &self.info {
            let end_rss = get_resident_set_size();
            let dur = info.start_time.elapsed();
            print_time_passes_entry(&info.message, dur, info.start_rss, end_rss, info.format);
        }
    }
}

struct JsonTimePassesEntry<'a> {
    pass: &'a str,
    time: f64,
    start_rss: Option<usize>,
    end_rss: Option<usize>,
}

impl Display for JsonTimePassesEntry<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { pass: what, time, start_rss, end_rss } = self;
        write!(f, r#"{{"pass":"{what}","time":{time},"rss_start":"#).unwrap();
        match start_rss {
            Some(rss) => write!(f, "{rss}")?,
            None => write!(f, "null")?,
        }
        write!(f, r#","rss_end":"#)?;
        match end_rss {
            Some(rss) => write!(f, "{rss}")?,
            None => write!(f, "null")?,
        }
        write!(f, "}}")?;
        Ok(())
    }
}

pub fn print_time_passes_entry(
    what: &str,
    dur: Duration,
    start_rss: Option<usize>,
    end_rss: Option<usize>,
    format: TimePassesFormat,
) {
    match format {
        TimePassesFormat::Json => {
            let entry =
                JsonTimePassesEntry { pass: what, time: dur.as_secs_f64(), start_rss, end_rss };

            eprintln!(r#"time: {entry}"#);
            return;
        }
        TimePassesFormat::Text => (),
    }

    // Print the pass if its duration is greater than 5 ms, or it changed the
    // measured RSS.
    let is_notable = || {
        if dur.as_millis() > 5 {
            return true;
        }

        if let (Some(start_rss), Some(end_rss)) = (start_rss, end_rss) {
            let change_rss = end_rss.abs_diff(start_rss);
            if change_rss > 0 {
                return true;
            }
        }

        false
    };
    if !is_notable() {
        return;
    }

    let rss_to_mb = |rss| (rss as f64 / 1_000_000.0).round() as usize;
    let rss_change_to_mb = |rss| (rss as f64 / 1_000_000.0).round() as i128;

    let mem_string = match (start_rss, end_rss) {
        (Some(start_rss), Some(end_rss)) => {
            let change_rss = end_rss as i128 - start_rss as i128;

            format!(
                "; rss: {:>4}MB -> {:>4}MB ({:>+5}MB)",
                rss_to_mb(start_rss),
                rss_to_mb(end_rss),
                rss_change_to_mb(change_rss),
            )
        }
        (Some(start_rss), None) => format!("; rss start: {:>4}MB", rss_to_mb(start_rss)),
        (None, Some(end_rss)) => format!("; rss end: {:>4}MB", rss_to_mb(end_rss)),
        (None, None) => String::new(),
    };

    eprintln!("time: {:>7}{}\t{}", duration_to_secs_str(dur), mem_string, what);
}

// Hack up our own formatting for the duration to make it easier for scripts
// to parse (always use the same number of decimal places and the same unit).
pub fn duration_to_secs_str(dur: std::time::Duration) -> String {
    format!("{:.3}", dur.as_secs_f64())
}

fn get_thread_id() -> u32 {
    std::thread::current().id().as_u64().get() as u32
}

// Memory reporting
cfg_select! {
    windows => {
        pub fn get_resident_set_size() -> Option<usize> {
            use windows::Win32::System::ProcessStatus::{
                K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
            };
            use windows::Win32::System::Threading::GetCurrentProcess;

            let mut pmc = PROCESS_MEMORY_COUNTERS::default();
            let pmc_size = size_of_val(&pmc);
            unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc_size as u32) }
                .ok()
                .ok()?;

            Some(pmc.WorkingSetSize)
        }
    }
    target_os = "macos" => {
        pub fn get_resident_set_size() -> Option<usize> {
            use std::mem;

            use libc::{PROC_PIDTASKINFO, c_int, c_void, getpid, proc_pidinfo, proc_taskinfo};
            const PROC_TASKINFO_SIZE: c_int = size_of::<proc_taskinfo>() as c_int;

            unsafe {
                let mut info: proc_taskinfo = mem::zeroed();
                let info_ptr = &mut info as *mut proc_taskinfo as *mut c_void;
                let pid = getpid() as c_int;
                let ret = proc_pidinfo(pid, PROC_PIDTASKINFO, 0, info_ptr, PROC_TASKINFO_SIZE);
                if ret == PROC_TASKINFO_SIZE { Some(info.pti_resident_size as usize) } else { None }
            }
        }
    }
    unix => {
        pub fn get_resident_set_size() -> Option<usize> {
            use libc::{_SC_PAGESIZE, sysconf};
            let field = 1;
            let contents = fs::read("/proc/self/statm").ok()?;
            let contents = String::from_utf8(contents).ok()?;
            let s = contents.split_whitespace().nth(field)?;
            let npages = s.parse::<usize>().ok()?;
            // SAFETY: `sysconf(_SC_PAGESIZE)` has no side effects and is safe to call.
            Some(npages * unsafe { sysconf(_SC_PAGESIZE) } as usize)
        }
    }
    _ => {
        pub fn get_resident_set_size() -> Option<usize> {
            None
        }
    }
}

#[cfg(test)]
mod tests;
