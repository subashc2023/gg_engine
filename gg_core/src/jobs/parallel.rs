use rayon::prelude::*;

/// Minimum entity count before parallelizing (overhead threshold).
pub const PAR_THRESHOLD: usize = 64;

/// Process items in parallel, returning transformed results.
/// Falls back to sequential iteration below `PAR_THRESHOLD`.
pub fn par_extract_map<T, R, P>(items: Vec<T>, process: P) -> Vec<R>
where
    T: Send,
    R: Send,
    P: Fn(T) -> R + Send + Sync,
{
    if items.len() < PAR_THRESHOLD {
        items.into_iter().map(process).collect()
    } else {
        super::pool().install(|| items.into_par_iter().map(process).collect())
    }
}

/// Process items in-place in parallel.
/// Falls back to sequential iteration below `PAR_THRESHOLD`.
pub fn par_for_each_mut<T, F>(items: &mut [T], f: F)
where
    T: Send,
    F: Fn(&mut T) + Send + Sync,
{
    if items.len() < PAR_THRESHOLD {
        items.iter_mut().for_each(f);
    } else {
        super::pool().install(|| items.par_iter_mut().for_each(f));
    }
}
