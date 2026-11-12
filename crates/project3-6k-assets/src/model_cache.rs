//! Bounded CPU model cache with generational handles.

use std::collections::HashMap;

use thiserror::Error;

use crate::CpuModelAsset;

/// Opaque slot and generation identity for a cached model.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ModelHandle {
    slot: usize,
    generation: u64,
}

/// Errors raised when inserting a model into the bounded CPU cache.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum ModelCacheError {
    #[error("model key must contain 1..={max_bytes} UTF-8 bytes")]
    InvalidKey { max_bytes: usize },
    #[error("model cache memory budget exceeded: {resident} + {requested} > {budget} bytes")]
    MemoryBudgetExceeded {
        resident: usize,
        requested: usize,
        budget: usize,
    },
    #[error("model cache asset count limit reached ({limit})")]
    AssetCountExceeded { limit: usize },
    #[error("model cache byte accounting overflowed")]
    SizeOverflow,
}

#[derive(Debug, Default)]
struct ModelSlot {
    generation: u64,
    model: Option<CpuModelAsset>,
    key: Option<String>,
}

/// Stores CPU models under stable keys while enforcing count and memory budgets.
#[derive(Debug)]
pub struct ModelAssetCache {
    max_assets: usize,
    max_key_bytes: usize,
    max_resident_bytes: usize,
    slots: Vec<ModelSlot>,
    free_slots: Vec<usize>,
    by_key: HashMap<String, ModelHandle>,
    resident_bytes: usize,
}

impl ModelAssetCache {
    /// Creates an empty model cache with explicit retained-memory and key-count bounds.
    pub fn new(max_assets: usize, max_key_bytes: usize, max_resident_bytes: usize) -> Self {
        Self {
            max_assets,
            max_key_bytes,
            max_resident_bytes,
            slots: Vec::new(),
            free_slots: Vec::new(),
            by_key: HashMap::new(),
            resident_bytes: 0,
        }
    }

    /// Inserts one imported model. A live key reuses its existing handle.
    pub fn insert(
        &mut self,
        key: &str,
        model: CpuModelAsset,
    ) -> Result<ModelHandle, ModelCacheError> {
        if key.is_empty() || key.len() > self.max_key_bytes {
            return Err(ModelCacheError::InvalidKey {
                max_bytes: self.max_key_bytes,
            });
        }
        if let Some(handle) = self.by_key.get(key).copied() {
            if self.model(handle).is_some() {
                return Ok(handle);
            }
            self.by_key.remove(key);
        }
        let new_resident_bytes = self
            .resident_bytes
            .checked_add(model.byte_size)
            .ok_or(ModelCacheError::SizeOverflow)?;
        if new_resident_bytes > self.max_resident_bytes {
            return Err(ModelCacheError::MemoryBudgetExceeded {
                resident: self.resident_bytes,
                requested: model.byte_size,
                budget: self.max_resident_bytes,
            });
        }
        if self.free_slots.is_empty() && self.slots.len() >= self.max_assets {
            return Err(ModelCacheError::AssetCountExceeded {
                limit: self.max_assets,
            });
        }
        let slot_index = self.free_slots.pop().unwrap_or_else(|| {
            self.slots.push(ModelSlot::default());
            self.slots.len() - 1
        });
        let handle = ModelHandle {
            slot: slot_index,
            generation: self.slots[slot_index].generation,
        };
        self.slots[slot_index].model = Some(model);
        self.slots[slot_index].key = Some(key.to_owned());
        self.by_key.insert(key.to_owned(), handle);
        self.resident_bytes = new_resident_bytes;
        Ok(handle)
    }

    /// Returns a model only while the handle generation is live.
    pub fn model(&self, handle: ModelHandle) -> Option<&CpuModelAsset> {
        let slot = self.slots.get(handle.slot)?;
        (slot.generation == handle.generation)
            .then_some(slot.model.as_ref())
            .flatten()
    }

    /// Removes a live model, invalidates its handle, and releases its retained-memory charge.
    pub fn unload(&mut self, handle: ModelHandle) -> bool {
        let Some(slot) = self.slots.get_mut(handle.slot) else {
            return false;
        };
        if slot.generation != handle.generation {
            return false;
        }
        let Some(model) = slot.model.take() else {
            return false;
        };
        self.resident_bytes -= model.byte_size;
        if let Some(key) = slot.key.take() {
            self.by_key.remove(&key);
        }
        if let Some(generation) = slot.generation.checked_add(1) {
            slot.generation = generation;
            self.free_slots.push(handle.slot);
        }
        true
    }

    /// Number of live cached models.
    pub fn asset_count(&self) -> usize {
        self.by_key.len()
    }

    /// CPU bytes charged to live models.
    pub fn resident_bytes(&self) -> usize {
        self.resident_bytes
    }
}

impl Default for ModelAssetCache {
    fn default() -> Self {
        Self::new(1024, 256, 256 * 1024 * 1024)
    }
}

#[cfg(test)]
mod tests {
    use super::{ModelAssetCache, ModelCacheError};
    use crate::CpuModelAsset;

    fn model(byte_size: usize) -> CpuModelAsset {
        CpuModelAsset {
            name: None,
            meshes: Vec::new(),
            instances: Vec::new(),
            materials: Vec::new(),
            images: Vec::new(),
            byte_size,
        }
    }

    #[test]
    fn cache_enforces_budget_and_invalidates_recycled_handles() {
        let mut cache = ModelAssetCache::new(1, 32, 8);
        let first = cache.insert("forest/tree", model(8)).unwrap();
        assert!(matches!(
            cache.insert("forest/rock", model(1)),
            Err(ModelCacheError::MemoryBudgetExceeded { .. })
        ));
        assert!(cache.unload(first));
        assert!(cache.model(first).is_none());
        let replacement = cache.insert("forest/rock", model(4)).unwrap();
        assert_eq!(first.slot, replacement.slot);
        assert_ne!(first.generation, replacement.generation);
        assert!(!cache.unload(first));
        assert_eq!(cache.resident_bytes(), 4);
    }
}
