use sha2::{Digest, Sha256};

/// Represents a Merkle Tree for storing cryptographic state commitments.
pub struct MerkleTree {
    /// The root hash of the tree.
    pub root: [u8; 32],
    /// The level of the tree (max 32 levels).
    pub levels: usize,
    /// The current leaf nodes/data inputs.
    data_leaves: Vec<Vec<u8>>,
}

impl MerkleTree {
    /// Creates a new, empty Merkle Tree.
    pub fn new(levels: usize) -> Self {
        MerkleTree {
            root: [0u8; 32],
            levels,
            data_leaves: Vec::new(),
        }
    }

    /// Builds the Merkle Tree from a provided set of data blocks.
    pub fn build(&mut self, leaves: Vec<Vec<u8>>) -> Result<(), &'static str> {
        if leaves.is_empty() {
            return Err("Cannot build tree from empty leaves.");
        }

        // Hash each leaf first
        let hashed: Vec<Vec<u8>> = leaves
            .iter()
            .map(|leaf| {
                let mut h = Sha256::new();
                h.update(leaf);
                h.finalize().to_vec()
            })
            .collect();

        self.data_leaves = leaves;
        self.root = Self::calculate_root_hash(hashed);
        Ok(())
    }

    /// Iteratively hashes adjacent node pairs up to the root level.
    /// Odd nodes are hashed with themselves (hash(x || x)).
    fn calculate_root_hash(mut current_level: Vec<Vec<u8>>) -> [u8; 32] {
        while current_level.len() > 1 {
            let mut next_level: Vec<Vec<u8>> = Vec::new();

            let mut i = 0;
            while i < current_level.len() {
                let left = &current_level[i];
                // If no right sibling, duplicate the left node
                let right = if i + 1 < current_level.len() {
                    &current_level[i + 1]
                } else {
                    left
                };

                let mut hasher = Sha256::new();
                hasher.update(left);
                hasher.update(right);
                next_level.push(hasher.finalize().to_vec());

                i += 2;
            }

            current_level = next_level;
        }

        let mut root = [0u8; 32];
        if let Some(r) = current_level.first() {
            root.copy_from_slice(r);
        }
        root
    }

    /// Gets the root hash as a hex string.
    pub fn get_root_hex(&self) -> String {
        hex::encode(self.root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_rejects_empty_leaves() {
        let mut tree = MerkleTree::new(32);
        assert!(tree.build(vec![]).is_err());
    }

    #[test]
    fn test_build_single_leaf() {
        let mut tree = MerkleTree::new(32);
        assert!(tree.build(vec![b"data1".to_vec()]).is_ok());
        // Root should be SHA256("data1")
        let expected = {
            let mut h = Sha256::new();
            h.update(b"data1");
            hex::encode(h.finalize())
        };
        assert_eq!(tree.get_root_hex(), expected);
    }

    #[test]
    fn test_build_even_leaves() {
        let mut tree = MerkleTree::new(32);
        let data = vec![b"data1".to_vec(), b"data2".to_vec()];
        assert!(tree.build(data).is_ok());
        assert_ne!(tree.root, [0u8; 32]);
    }

    #[test]
    fn test_build_odd_leaves() {
        let mut tree = MerkleTree::new(32);
        let data = vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()];
        assert!(tree.build(data).is_ok());
        assert_ne!(tree.root, [0u8; 32]);
    }

    #[test]
    fn test_deterministic_root() {
        let data = vec![b"x".to_vec(), b"y".to_vec()];
        let mut t1 = MerkleTree::new(32);
        let mut t2 = MerkleTree::new(32);
        t1.build(data.clone()).unwrap();
        t2.build(data).unwrap();
        assert_eq!(t1.root, t2.root);
    }
}
