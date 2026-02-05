use rayon::prelude::*;

pub fn run_in_parallel<T, R, F>(items: Vec<T>, jobs: Option<usize>, func: F) -> Vec<R>
where
    T: Send,
    R: Send,
    F: Fn(T) -> R + Send + Sync,
{
    match jobs {
        Some(count) if count > 1 => {
            let pool = rayon::ThreadPoolBuilder::new().num_threads(count).build();
            if let Ok(pool) = pool {
                return pool.install(|| items.into_par_iter().map(func).collect());
            }
            items.into_iter().map(func).collect()
        }
        _ => items.into_iter().map(func).collect(),
    }
}
