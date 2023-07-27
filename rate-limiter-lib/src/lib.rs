use chrono::{DateTime, Duration, Utc};
use evmap::{ReadHandle, ReadHandleFactory, WriteHandle};
use parking_lot::Mutex;
use priority_queue::double_priority_queue::DoublePriorityQueue;
use std::{error::Error, fmt, sync::Arc};
use tokio::{task, task::JoinHandle};

#[derive(Debug)]
pub enum ModelError {
    NotFound,
    AlreadyPresent,
    PastRateLimit(i64),
}

pub type KeyType = String;
pub type LimitType = i64;
pub type InternalValue = Box<StoredValue>;

#[derive(Eq, PartialEq, Hash, Clone)]
pub struct StoredValue {
    pub count: LimitType,
    pub ttl: Option<DateTime<Utc>>,
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelError::NotFound => write!(f, "Key Not Found"),
            ModelError::AlreadyPresent => write!(f, "Key is not present in the data set"),
            ModelError::PastRateLimit(time_remaining) => {
                write!(f, "Rate limit exceeded please wait {} seconds", time_remaining)
            },
        }
    }
}

impl Error for ModelError {}

pub struct Store {}

impl Store {
    /// If the counter is below its associated limit increment it. If/When the limit is reached
    /// then calculate the wait time until the rate limit counter has expired and return
    /// Err<ModelError> to the api layer
    pub fn inc_below_limit(
        writer_m: &Mutex<WriteHandle<KeyType, InternalValue>>,
        reader: &ReadHandle<KeyType, InternalValue>,
        key: KeyType,
        limit: LimitType,
        ttl: i64,
    ) -> Result<(), ModelError> {
        if let Some(mut stored_value) = Self::get(reader, &key)? {
            if stored_value.count < limit {
                stored_value.count += 1;
                // re-add the same stored_value to keep ttl
                Self::upsert_stored_type(writer_m, key, stored_value)?;
            } else {
                let time_remaining = stored_value
                    .ttl
                    .map(|ttl| ttl.signed_duration_since(Utc::now()).num_seconds())
                    .unwrap_or_default();
                return Err(ModelError::PastRateLimit(time_remaining));
            }
        } else {
            Self::insert(writer_m, &key, 1_i64, ttl)?;
        }
        Ok(())
    }

    /// This upsert function is a workaround since individual elements in EvMaps are not mutable
    /// for the sake of consistencny. Instead of direct mutation remove the element and re-add with
    /// the same ttl and incremenented count. In order to avoid race conditions the EvMap is then
    /// refreshed this has a very small chance of conflicting with the loop that reconcilles EvMap
    /// state which could deadlock. In a production scale project this should probably be owned by
    /// a single actor.
    fn upsert_stored_type(
        writer_m: &Mutex<WriteHandle<KeyType, InternalValue>>,
        key: KeyType,
        stored_value: StoredValue,
    ) -> Result<(), ModelError> {
        let mut writer = writer_m.lock();
        writer.empty(key.to_owned());
        writer.insert(key, Box::new(stored_value));
        writer.refresh();
        Ok(())
    }

    pub fn insert(
        writer_m: &Mutex<WriteHandle<KeyType, InternalValue>>,
        key: &KeyType,
        count: LimitType,
        ttl: i64,
    ) -> Result<(), ModelError> {
        let mut writer = writer_m.lock();
        let current_ttl = Utc::now() + Duration::seconds(ttl);
        if writer.contains_key(key) {
            return Err(ModelError::AlreadyPresent);
        } else {
            writer.insert(
                key.to_owned(),
                Box::new(StoredValue {
                    count,
                    ttl: Some(current_ttl),
                }),
            );
        }
        Ok(())
    }

    pub fn delete(writer_m: &Mutex<WriteHandle<KeyType, InternalValue>>, key: &KeyType) -> Result<(), ModelError> {
        let mut writer = writer_m.lock();
        if !writer.contains_key(key) {
            return Err(ModelError::NotFound);
        }
        writer.empty(key.to_owned());
        Ok(())
    }

    pub fn get(reader: &ReadHandle<KeyType, InternalValue>, key: &KeyType) -> Result<Option<StoredValue>, ModelError> {
        Ok(reader.get_one(key).map(|v| *v.clone()))
    }
    
    /// This is the main loop for the in memory store. It will iterate the in memory EvMap removing
    /// elements past their ttl if a ttl has been set. To make this process more efficient rather
    /// that searching the structure for past TTLs push item ttl onto a queue when added then
    /// continuously pop items off the queue and remove them from the EvMap.
    pub async fn init() -> (
        ReadHandleFactory<KeyType, InternalValue>,
        Arc<Mutex<WriteHandle<KeyType, InternalValue>>>,
        JoinHandle<()>,
    ) {
        let (read_handle, mut write_handle): (ReadHandle<KeyType, InternalValue>, WriteHandle<KeyType, InternalValue>) =
            evmap::new();
        // initiall call used so that we can get accurate pending transactions
        // https://docs.rs/evmap/latest/evmap/struct.WriteHandle.html#method.pending
        write_handle.refresh();
        let writer = Arc::new(Mutex::new(write_handle));
        let internal_writer = writer.clone();
        let mut ttl_queue: DoublePriorityQueue<KeyType, DateTime<Utc>> = DoublePriorityQueue::new();
        let timer_handler = task::spawn(async move {
            loop {
                let mut write_handle = internal_writer.lock();
                for operation in write_handle.pending() {
                    match operation {
                        evmap::Operation::Add(k, v) => {
                            if let Some(ttl) = v.ttl {
                                ttl_queue.push(k.clone(), ttl);
                            }
                        },
                        evmap::Operation::Empty(k) => {
                            if ttl_queue.get(k).is_some() {
                                ttl_queue.remove(k);
                            }
                        },
                        _ => (),
                    }
                }
                let mut next_ttl = ttl_queue.peek_min();
                while next_ttl.is_some() && Utc::now() > *next_ttl.unwrap().1 {
                    write_handle.empty(next_ttl.unwrap().0.clone());
                    ttl_queue.pop_min();
                    next_ttl = ttl_queue.peek_min();
                }
                write_handle.refresh();
                #[cfg(test)]
                // wait for queue to clear for ttl testing
                if ttl_queue.is_empty() {
                    break;
                }
            }
        });
        (read_handle.factory(), writer, timer_handler)
    }
}
