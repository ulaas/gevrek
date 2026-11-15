use serde::{Deserialize, Serialize};

/// A circular buffer that provides O(1) focus operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ring<T> {
    elements: Vec<T>,
    focused_idx: usize,
}

impl<T> Ring<T> {
    pub fn new() -> Self {
        Self {
            elements: Vec::new(),
            focused_idx: 0,
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            elements: Vec::with_capacity(capacity),
            focused_idx: 0,
        }
    }

    /// Push element to the end
    pub fn push(&mut self, element: T) {
        self.elements.push(element);
        if self.elements.len() == 1 {
            self.focused_idx = 0;
        }
    }

    /// Push element to the front (index 0)
    pub fn push_front(&mut self, element: T) {
        self.elements.insert(0, element);
        // Adjust focus index since we inserted at the front
        if self.elements.len() > 1 {
            self.focused_idx += 1;
        }
    }

    /// Pop element from the back (last element)
    pub fn pop_back(&mut self) -> Option<T> {
        if self.elements.is_empty() {
            return None;
        }
        let removed = self.elements.pop();
        // Adjust focus if necessary
        if self.focused_idx >= self.elements.len() && !self.elements.is_empty() {
            self.focused_idx = self.elements.len() - 1;
        }
        removed
    }

    /// Insert element at focused position
    pub fn insert(&mut self, element: T) {
        if self.elements.is_empty() {
            self.push(element);
        } else {
            self.elements.insert(self.focused_idx, element);
        }
    }

    /// Remove element at focused position
    pub fn remove(&mut self) -> Option<T> {
        if self.elements.is_empty() {
            return None;
        }

        let removed = self.elements.remove(self.focused_idx);

        // Adjust focus after removal
        if self.elements.is_empty() {
            self.focused_idx = 0;
        } else if self.focused_idx >= self.elements.len() {
            self.focused_idx = self.elements.len() - 1;
        }

        Some(removed)
    }

    /// Remove element at specific index
    pub fn remove_at(&mut self, idx: usize) -> Option<T> {
        if idx >= self.elements.len() {
            return None;
        }

        let removed = self.elements.remove(idx);

        // Adjust focus if necessary
        if self.elements.is_empty() {
            self.focused_idx = 0;
        } else if idx < self.focused_idx && self.focused_idx > 0 {
            // Element removed before focus - shift focus back to stay on same element
            self.focused_idx -= 1;
        } else if self.focused_idx >= self.elements.len() {
            // Focus was at or past the end - wrap to last element
            self.focused_idx = self.elements.len() - 1;
        }
        // When idx == focused_idx, the focused element was removed.
        // focused_idx now points at the next element (the one that shifted into this slot),
        // which is the intended behavior (focus advances forward).

        Some(removed)
    }

    /// Insert element at specific index
    pub fn insert_at(&mut self, idx: usize, element: T) {
        let insert_idx = idx.min(self.elements.len());
        self.elements.insert(insert_idx, element);

        // Adjust focus if insertion happened before or at focused position
        if insert_idx <= self.focused_idx {
            self.focused_idx += 1;
        }

        // If this is the first element, set focus to it
        if self.elements.len() == 1 {
            self.focused_idx = 0;
        }
    }

    /// Focus next element (cycle forward)
    pub fn focus_next(&mut self) {
        if !self.elements.is_empty() {
            self.focused_idx = (self.focused_idx + 1) % self.elements.len();
        }
    }

    /// Focus previous element (cycle backward)
    pub fn focus_previous(&mut self) {
        if !self.elements.is_empty() {
            self.focused_idx = if self.focused_idx == 0 {
                self.elements.len() - 1
            } else {
                self.focused_idx - 1
            };
        }
    }

    /// Swap focused element with next element (cycle forward)
    pub fn swap_with_next(&mut self) {
        if self.elements.len() > 1 {
            let next_idx = (self.focused_idx + 1) % self.elements.len();
            self.elements.swap(self.focused_idx, next_idx);
            self.focused_idx = next_idx;
        }
    }

    /// Swap focused element with previous element (cycle backward)
    pub fn swap_with_previous(&mut self) {
        if self.elements.len() > 1 {
            let prev_idx = if self.focused_idx == 0 {
                self.elements.len() - 1
            } else {
                self.focused_idx - 1
            };
            self.elements.swap(self.focused_idx, prev_idx);
            self.focused_idx = prev_idx;
        }
    }

    /// Set focus to specific index
    pub fn focus_at(&mut self, idx: usize) -> bool {
        if idx < self.elements.len() {
            self.focused_idx = idx;
            true
        } else {
            false
        }
    }

    /// Get currently focused element
    pub fn focused(&self) -> Option<&T> {
        self.elements.get(self.focused_idx)
    }

    /// Get mutable reference to focused element
    pub fn focused_mut(&mut self) -> Option<&mut T> {
        self.elements.get_mut(self.focused_idx)
    }

    /// Get focused index
    pub fn focused_idx(&self) -> usize {
        self.focused_idx
    }

    /// Get element at index
    pub fn get(&self, idx: usize) -> Option<&T> {
        self.elements.get(idx)
    }

    /// Get mutable element at index
    pub fn get_mut(&mut self, idx: usize) -> Option<&mut T> {
        self.elements.get_mut(idx)
    }

    /// Get all elements
    pub fn elements(&self) -> &[T] {
        &self.elements
    }

    /// Get mutable elements
    pub fn elements_mut(&mut self) -> &mut Vec<T> {
        &mut self.elements
    }

    /// Number of elements
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// Clear all elements
    pub fn clear(&mut self) {
        self.elements.clear();
        self.focused_idx = 0;
    }

    /// Iterate over elements
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.elements.iter()
    }

    /// Iterate mutably over elements
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.elements.iter_mut()
    }
}

impl<T> Default for Ring<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> FromIterator<T> for Ring<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let elements: Vec<T> = iter.into_iter().collect();
        Self {
            focused_idx: 0,
            elements,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_and_focus() {
        let mut ring = Ring::new();
        ring.push(1);
        ring.push(2);
        ring.push(3);

        assert_eq!(ring.focused(), Some(&1));
        ring.focus_next();
        assert_eq!(ring.focused(), Some(&2));
        ring.focus_next();
        assert_eq!(ring.focused(), Some(&3));
        ring.focus_next(); // Wraps around
        assert_eq!(ring.focused(), Some(&1));
    }

    #[test]
    fn test_focus_previous() {
        let mut ring = Ring::new();
        ring.push(1);
        ring.push(2);
        ring.push(3);

        assert_eq!(ring.focused(), Some(&1));
        ring.focus_previous(); // Wraps to end
        assert_eq!(ring.focused(), Some(&3));
        ring.focus_previous();
        assert_eq!(ring.focused(), Some(&2));
    }

    #[test]
    fn test_remove() {
        let mut ring = Ring::new();
        ring.push(1);
        ring.push(2);
        ring.push(3);

        ring.focus_at(1);
        assert_eq!(ring.focused(), Some(&2));

        let removed = ring.remove();
        assert_eq!(removed, Some(2));
        assert_eq!(ring.len(), 2);
        assert_eq!(ring.focused(), Some(&3));
    }
}
