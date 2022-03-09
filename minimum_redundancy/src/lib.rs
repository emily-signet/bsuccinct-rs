#![doc = include_str!("../README.md")]

use std::collections::HashMap;
use std::hash::Hash;
use co_sort::{Permutation, co_sort};

use std::borrow::Borrow;
use dyn_size_of::GetSize;

mod code;
pub use code::Code;
mod frequencies;
pub use frequencies::Frequencies;
mod degree;
pub use degree::*;
mod io;
pub use io::*;

/// Succinct representation of minimum-redundancy coding
/// (huffman tree of some degree in the canonical form).
pub struct Coding<ValueType, D> {
    /// Values, from the most frequent to the least.
    pub values: Box<[ValueType]>,
    /// Number of the internal nodes of each tree level. The root is not counted.
    /// Contains exactly one zero at the end.
    pub internal_nodes_count: Box<[u32]>,
    /// Size of the fragment given as bits per fragment or the degree of the Huffman tree.
    pub degree: D
}

/// Points the number of bytes needed to store value of the type `ValueType`.
/// This number of bytes can be constant or can depend on the value.
pub enum ValueSize<ValueType> {
    /// Holds constant number of bytes needed to store value.
    Const(usize),
    /// Holds callback that shows the number of bytes needed to store given value.
    Variable(Box<dyn Fn(&ValueType)->usize>)
}

impl<ValueType: GetSize, D> dyn_size_of::GetSize for Coding<ValueType, D> {
    fn size_bytes_dyn(&self) -> usize {
        self.values.size_bytes_dyn() + self.internal_nodes_count.size_bytes_dyn()
    }
    const USES_DYN_MEM: bool = true;
}

impl<ValueType, D: TreeDegree> Coding<ValueType, D> {

    /// Constructs coding for given `frequencies` of values and `degree` of the Huffman tree.
    pub fn from_frequencies<F: Frequencies<Value=ValueType>>(degree: D, frequencies: F) -> Self {
        let (values, mut freq) = frequencies.into_sorted();
        Self::from_sorted(degree, values, &mut freq)
    }

    /// Counts occurrences of all values exposed by `iter` and constructs coding for obtained
    /// frequencies of values and `degree` of the Huffman tree.
    pub fn from_iter<Iter>(degree: D, iter: Iter) -> Self
        where Iter: IntoIterator, Iter::Item: Borrow<ValueType>, ValueType: Hash + Eq + Clone
    {
        Self::from_frequencies(degree, HashMap::<ValueType, u32>::with_counted_all(iter))
    }

    /// Returns total (summarized) number of code fragments of all values.
    ///
    /// The algorithm runs in *O(L)* time and *O(1)* memory,
    /// where *L* is the number of fragments in the longest codeword.
    pub fn total_fragments_count(&self) -> usize {
        let mut values_to_count = self.values.len();
        let mut result = 0;
        let mut prev_internal = 1;
        for (level, curr_internal) in self.internal_nodes_count.iter().enumerate() {
            let curr_total = self.degree * prev_internal;
            prev_internal = *curr_internal;
            let curr_leaf = ((curr_total - *curr_internal) as usize).min(values_to_count);
            values_to_count -= curr_leaf;
            result += curr_leaf * (level+1);
        }
        result
    }

    /// Returns decoder that allows for decoding a value.
    #[inline] pub fn decoder(&self) -> Decoder<ValueType, D> {
        return Decoder::<ValueType, D>::new(self);
    }

    /// Construct coding (of given `degree`) for the given `values`, where
    /// `freq` is an array of numbers of occurrences of corresponding values.
    /// `freq` has to be in non-descending order and of the same length as values.
    ///
    /// The algorithm runs in *O(values.len)* time,
    /// in-place (it uses and changes `freq` and move values to the returned `Coding` object).
    pub fn from_sorted(degree: D, mut values: Box<[ValueType]>, freq: &mut [u32]) -> Self {
        let len = freq.len();
        let tree_degree = degree.as_u32();
        if len <= tree_degree as usize {
            values.reverse();
            return Coding {
                values,
                internal_nodes_count: vec![0u32].into_boxed_slice(),
                degree,
            }
        }

        let reduction_per_step = tree_degree - 1;

        let mut current_tree_degree = ((len - 1) % reduction_per_step as usize) as u32; // desired reduction in the first step
        current_tree_degree = if current_tree_degree == 0 { tree_degree } else { current_tree_degree + 1 }; // children of the internal node constructed in the first step
        let mut internals_begin = 0usize; // first internal node = next parent node to be used
        let mut leafs_begin = 0usize;     // next leaf to be used
        let internal_nodes_size = (len + reduction_per_step as usize - 2) / reduction_per_step as usize;    // (len-1) / reduction_per_step rounded up

        for next in 0..internal_nodes_size {    // next is next value to be assigned
            // select first item for a pairing
            if leafs_begin >= len || freq[internals_begin as usize] < freq[leafs_begin] {
                freq[next] = freq[internals_begin];
                freq[internals_begin] = next as u32;
                internals_begin += 1;
            } else {
                freq[next] = freq[leafs_begin];
                leafs_begin += 1;
            }

            // add on the second, ... items
            for _ in 1..current_tree_degree {
                if leafs_begin >= len || (internals_begin < next && freq[internals_begin] < freq[leafs_begin]) {
                    freq[next] += freq[internals_begin];
                    freq[internals_begin] = next as u32;
                    internals_begin += 1;
                } else {
                    freq[next] += freq[leafs_begin];
                    leafs_begin += 1;
                }
            }
            current_tree_degree = tree_degree;    // only in first iteration can be: current_tree_degree != tree_degree
        }
        //dbg!(&freq);
        //dbg!(&internal_nodes_size);
        // second pass, right to left, setting internal depths, we also find the maximum depth
        let mut max_depth = 0u8;
        freq[internal_nodes_size - 1] = 0;    // value for the root
        for next in (0..internal_nodes_size - 1).rev() {
            freq[next] = freq[freq[next] as usize] + 1;
            if freq[next] as u8 > max_depth { max_depth = freq[next] as u8; }
        }

        values.reverse();
        let mut result = Self {
            values,
            internal_nodes_count: vec![0u32; max_depth as usize + 1].into_boxed_slice(),
            degree
        };
        for i in 0..internal_nodes_size - 1 {
            result.internal_nodes_count[freq[i] as usize - 1] += 1;  // only root is at the level 0, we skip it
        }   // no internal nodes at the last level, result.internal_nodes_count[max_depth] is 0

        return result;
    }

    /// Construct coding (of the given `degree`) for the given `values`, where
    /// `freq` has to be of the same length as values and contain number of occurrences of corresponding values.
    ///
    /// The algorithm runs in *O(values.len * log(values.len))* time.
    pub fn from_unsorted(degree: D, mut values: Box<[ValueType]>, freq: &mut [u32]) -> Self{
        co_sort!(freq, values);
        Self::from_sorted(degree, values, freq)
    }

    /// Returns number of bytes which `write_internal_nodes_count` will write.
    pub fn write_internal_nodes_count_bytes(&self) -> usize {
        let l = self.internal_nodes_count.len()-1;
        vbyte_len(l as u32) as usize
            + self.internal_nodes_count[..l].iter().map(|v| vbyte_len(*v) as usize).sum::<usize>()
    }

    /// Writes `internal_nodes_count` to `output` as the following `internal_nodes_count.len()`, VByte values:
    /// `internal_nodes_count.len()-1` (=l), `internal_nodes_count[0]`, `internal_nodes_count[1]`, ..., `internal_nodes_count[l-1]`
    pub fn write_internal_nodes_count(&self, output: &mut dyn std::io::Write) -> std::io::Result<()> {
        let l = self.internal_nodes_count.len()-1;
        vbyte_write(output, l as u32)?;
        self.internal_nodes_count[..l].iter().try_for_each(|v| vbyte_write(output, *v))
    }

    /// Reads (written by `write_internal_nodes_count`) `internal_nodes_count` from `input`.
    pub fn read_internal_nodes_count(input: &mut dyn std::io::Read) -> std::io::Result<Box<[u32]>> {
        let s = vbyte_read(input)?;
        let mut v = Vec::with_capacity(s as usize + 1);
        for _ in 0..s { v.push(vbyte_read(input)?); }
        v.push(0);
        Ok(v.into_boxed_slice())
    }

    /// Returns number of bytes which `write_values` will write,
    /// assuming that each call to `write_value` writes the number of bytes pointed by `value_size`.
    pub fn write_values_size_bytes(&self, value_size: ValueSize<ValueType>) -> usize {
        vbyte_len(self.values.len() as u32) as usize +
            match value_size {
                ValueSize::Const(bytes_per_value) => { bytes_per_value*self.values.len() }
                ValueSize::Variable(f) => { self.values.iter().map(|v| f(v)).sum::<usize>() }
            }
    }

    /// Writes `values` to the given `output`, using `write_value` to write each value.
    pub fn write_values<F>(&self, output: &mut dyn std::io::Write, mut write_value: F) -> std::io::Result<()>
        where F: FnMut(&mut dyn std::io::Write, &ValueType) -> std::io::Result<()>
    {
        vbyte_write(output, self.values.len() as u32)?;
        self.values.iter().try_for_each(|v| { write_value(output, v) })
    }

    /// Reads `values` from the given `input`, using `read_value` to read each value.
    pub fn read_values<F>(input: &mut dyn std::io::Read, mut read_value: F) -> std::io::Result<Box<[ValueType]>>
        where F: FnMut(&mut dyn std::io::Read) -> std::io::Result<ValueType>
    {
        let s = vbyte_read(input)?;
        let mut v = Vec::with_capacity(s as usize);
        for _ in 0..s { v.push(read_value(input)?); }
        Ok(v.into_boxed_slice())
    }

    /// Returns number of bytes which `write` will write,
    /// assuming that each call to `write_value` writes the number of bytes pointed by `value_size`.
    pub fn write_size_bytes(&self, value_size: ValueSize<ValueType>) -> usize {
        self.degree.write_size_bytes() + self.write_internal_nodes_count_bytes()
            + self.write_values_size_bytes(value_size)
    }

    /// Writes `self` to the given `output`, using `write_value` to write each value.
    pub fn write<F>(&self, output: &mut dyn std::io::Write, write_value: F) -> std::io::Result<()>
        where F: FnMut(&mut dyn std::io::Write, &ValueType) -> std::io::Result<()>
    {
        self.degree.write(output)?;
        self.write_internal_nodes_count(output)?;
        self.write_values(output, write_value)
    }

    /// Reads `Coding` from the given `input`, using `read_value` to read each value.
    pub fn read<F>(input: &mut dyn std::io::Read, read_value: F) -> std::io::Result<Self>
        where F: FnMut(&mut dyn std::io::Read) -> std::io::Result<ValueType>
    {
        let fragment_size = D::read(input)?;
        let internal_nodes_count = Self::read_internal_nodes_count(input)?;
        Ok(Self {
            values: Self::read_values(input, read_value)?,
            internal_nodes_count,
            degree: fragment_size
        })
    }

    /// Calls `f` for each level in the huffman tree.
    /// Arguments of `f` are: values assigned to the leafs at the current level, index of level in the tree (counting from 0),
    /// number of internal nodes at the level (which equals the index of the first leaf at the level).
    pub fn for_each_level<F>(&self, mut f: F)    // TODO reimplement to be a normal iterator
        where F: FnMut(&[ValueType], u32, u32)  //values, level index, number of internal nodes = index of the first leaf at the level
    {
        let mut level_size = self.degree.as_u32();
        let mut value_index = 0usize;
        for level in 0u32..self.internal_nodes_count.len() as u32 {
            let internal_nodes = self.internal_nodes_count[level as usize];
            let leaves_count = level_size - internal_nodes;
            let last_value_index = value_index + leaves_count as usize;
            if self.values.len() <= last_value_index {
                f(&self.values[value_index..], level, internal_nodes);
                return;
            } else {
                f(&self.values[value_index..last_value_index], level, internal_nodes);
            }
            value_index = last_value_index;
            level_size = self.degree * internal_nodes;
        }
    }

    /// Calls `f` for each leaf in the huffman tree.
    /// Arguments of `f` are: value assigned to the leaf, level of leaf in the tree (counting from 0),
    /// number of internal nodes at the level, index of leaf at the level.
    pub fn for_each_leaf<F>(&self, mut f: F)    // TODO reimplement to be a normal iterator
        where F: FnMut(&ValueType, u32, u32, u32)  //value: &ValueType, level: u32, internal_nodes: u32, leaf_index: u32
    {
        self.for_each_level(|values, level, internal_nodes| {
            for (i, v) in values.iter().enumerate() {
                f(v, level, internal_nodes, i as u32)
            }
        })
    }
}

impl<ValueType: Hash + Eq + Clone, D: TreeDegree> Coding<ValueType, D> {

    /// Returns a map from values to the lengths of their codes.
    pub fn fragment_counts_for_values(&self) -> HashMap<ValueType, u32> {
        let mut result = HashMap::<ValueType, u32>::with_capacity(self.values.len());
        self.for_each_leaf(|value, level, _, _| { result.insert(value.clone(), level + 1); });
        return result;
    }

    /// Returns a map from values to their codes.
    pub fn codes_for_values(&self) -> HashMap<ValueType, Code> {
        // fill map for encoding:
        let mut result = HashMap::<ValueType, Code>::with_capacity(self.values.len());
        self.for_each_leaf(|value, level, internal_nodes, leaf_index| {
            result.insert(value.clone(),
                          Code { bits: internal_nodes + leaf_index, fragments: level + 1 });
        });
        return result;
    }
}

/// Result of fragment decoding returned be `consume` method of `Decoder`.
#[derive(PartialOrd, Ord, PartialEq, Eq, Debug, Clone, Hash)]
pub enum DecodingResult<T> {
    /// Completed value that has been successfully decoded.
    Value(T),
    /// The codeword is incomplete and the next fragment is needed.
    Incomplete,
    /// The codeword is invalid (possible only for bits per fragment > 1).
    Invalid
}

impl<T> From<Option<T>> for DecodingResult<T> {
    #[inline(always)] fn from(option: Option<T>) -> Self {
        if let Some(v) = option { DecodingResult::Value(v) } else { DecodingResult::Invalid }
    }
}

/// Decoder that decodes a value for given code, consuming one codeword fragment at a time.
///
/// Time complexity of decoding the whole code is:
/// - pessimistic: *O(length of the longest code)*
/// - expected: *O(log(number of values, i.e. length of coding.values))*
/// - optimistic: *O(1)*
///
/// Memory complexity: *O(1)*
pub struct Decoder<'huff, ValueType, D> {
    coding: &'huff Coding<ValueType, D>,
    // shift+fragment is a current position (node number, counting from the left) at current level
    shift: u32,
    // number of leafs at all previous levels
    first_leaf_nr: u32,
    // current level size = number of: internal nodes + leaves
    level_size: u32,
    // number of the current level
    level: u8
}   // Note: Brodnik describes also faster decoder that runs in expected loglog(length of the longest code) expected time, but requires all codeword bits in advance.

impl<'huff, ValueType, D: TreeDegree> Decoder<'huff, ValueType, D> {
    /// Constructs decoder for given `coding`.
    pub fn new(coding: &'huff Coding<ValueType, D>) -> Self {
        Self {
            coding,
            shift: 0,
            first_leaf_nr: 0,
            level_size: coding.degree.as_u32(),
            level: 0
        }
    }

    /// Consumes a `fragment` of the codeword and returns:
    /// - a value if the given `fragment` finishes the valid codeword;
    /// - an `DecodingResult::Incomplete` if the codeword is incomplete and the next fragment is needed;
    /// - or `DecodingResult::Invalid` if the codeword is invalid (possible only for bits per fragment > 1).
    ///
    /// Result is undefined if `fragment` exceeds `tree_degree`.
    pub fn consume(&mut self, fragment: u32) -> DecodingResult<&'huff ValueType> {
        self.shift += fragment;
        let internal_nodes_count = self.internal_nodes_count();
        return if self.shift < internal_nodes_count {    // internal node, go level down
            //self.shift *= self.coding.tree_degree as u32;
            //self.shift <<= self.coding.bits_per_fragment;
            self.shift = self.coding.degree * self.shift;
            self.first_leaf_nr += self.level_size - internal_nodes_count;    // increase by number of leafs at current level
            //self.level_size = internal_nodes_count * self.coding.tree_degree as u32; // size of the next level
            //self.level_size = internal_nodes_count << self.coding.bits_per_fragment; // size of the next level
            self.level_size = self.coding.degree * internal_nodes_count;
                self.level += 1;
            DecodingResult::Incomplete
        } else {    // leaf, return value or Invalid
            self.coding.values.get((self.first_leaf_nr + self.shift - internal_nodes_count) as usize).into()
            //self.coding.values.get((self.first_leaf_nr + self.level_size + self.shift) as usize).into()
        }
    }

    /// Consumes a `fragment` of the codeword and returns:
    /// - a value if the given `fragment` finishes the valid codeword;
    /// - an `DecodingResult::Incomplete` if the codeword is incomplete and the next fragment is needed;
    /// - or `DecodingResult::Invalid` if the codeword is invalid (possible only for `degree` greater than 2)
    ///     or `fragment` exceeds `degree`.
    #[inline(always)] pub fn consume_checked(&mut self, fragment: u32) -> DecodingResult<&'huff ValueType> {
        if fragment < self.coding.degree.as_u32() {
            self.consume(fragment)
        } else {
            DecodingResult::Invalid
        }
    }

    /// Returns number of internal (i.e. non-leafs) nodes at the current level of the tree.
    #[inline(always)] fn internal_nodes_count(&self) -> u32 {
        self.coding.internal_nodes_count[self.level as usize]
    }
}

/// Heuristically calculates bits per fragment that gives about constant length average code size.
/// `entropy` should equals to entropy or a bit less, e.g. entropy minus `0.2`
pub fn entropy_to_bpf(entropy: f64) -> u8 {
    (1f64.max(entropy).ceil() as u64 - 1).min(8) as u8
}

#[cfg(test)]
mod tests {
    // Note this useful idiom: importing names from outer (for mod tests) scope.
    use super::*;
    use maplit::hashmap;

    fn test_read_write<FS: TreeDegree>(huffman: &Coding<char, FS>) {
        let mut buff = Vec::new();
        huffman.write_values(&mut buff, |b, v| write_int!(b, *v as u8)).unwrap();
        assert_eq!(buff.len(), huffman.write_values_size_bytes(ValueSize::Const(1)));
        assert_eq!(Coding::<_, FS>::read_values(&mut &buff[..], |b| read_int!(b, u8).map(|v| v as char)).unwrap(), huffman.values);
        buff.clear();
        huffman.write_internal_nodes_count(&mut buff).unwrap();
        assert_eq!(buff.len(), huffman.write_internal_nodes_count_bytes());
        assert_eq!(Coding::<char, FS>::read_internal_nodes_count(&mut &buff[..]).unwrap(), huffman.internal_nodes_count);
        buff.clear();
        huffman.write(&mut buff, |b, v| write_int!(b, *v as u8)).unwrap();
        assert_eq!(buff.len(), huffman.write_size_bytes(ValueSize::Const(1)));
        let read = Coding::<_, FS>::read(&mut &buff[..], |b| read_int!(b, u8).map(|v| v as char)).unwrap();
        assert_eq!(huffman.degree.as_u32(), read.degree.as_u32());
        assert_eq!(huffman.values, read.values);
        assert_eq!(huffman.internal_nodes_count, read.internal_nodes_count);
    }

    #[test]
    fn coding_3sym_1bit() {
        //  /  \
        // /\  a
        // bc
        let huffman = Coding::from_frequencies(BitsPerFragment(1),
                                               hashmap!('a' => 100, 'b' => 50, 'c' => 10));
        assert_eq!(huffman.total_fragments_count(), 5);
        assert_eq!(huffman.values.as_ref(), ['a', 'b', 'c']);
        assert_eq!(huffman.internal_nodes_count.as_ref(), [1, 0]);
        assert_eq!(huffman.codes_for_values(), hashmap!(
                'a' => Code{ bits: 0b1, fragments: 1 },
                'b' => Code{ bits: 0b00, fragments: 2 },
                'c' => Code{ bits: 0b01, fragments: 2 }
               ));
        let mut decoder_for_a = huffman.decoder();
        assert_eq!(decoder_for_a.consume(1), DecodingResult::Value(&'a'));
        let mut decoder_for_b = huffman.decoder();
        assert_eq!(decoder_for_b.consume(0), DecodingResult::Incomplete);
        assert_eq!(decoder_for_b.consume(0), DecodingResult::Value(&'b'));
        let mut decoder_for_c = huffman.decoder();
        assert_eq!(decoder_for_c.consume(0), DecodingResult::Incomplete);
        assert_eq!(decoder_for_c.consume(1), DecodingResult::Value(&'c'));
        test_read_write(&huffman);
    }

    #[test]
    fn coding_3sym_2bits() {
        //  /|\
        //  abc
        let huffman = Coding::from_frequencies(BitsPerFragment(2),
                                               hashmap!('a' => 100, 'b' => 50, 'c' => 10));
        assert_eq!(huffman.total_fragments_count(), 3);
        assert_eq!(huffman.values.as_ref(), ['a', 'b', 'c']);
        assert_eq!(huffman.internal_nodes_count.as_ref(), [0]);
        assert_eq!(huffman.codes_for_values(), hashmap!(
                'a' => Code{ bits: 0, fragments: 1 },
                'b' => Code{ bits: 1, fragments: 1 },
                'c' => Code{ bits: 2, fragments: 1 }
               ));
        let mut decoder_for_a = huffman.decoder();
        assert_eq!(decoder_for_a.consume(0), DecodingResult::Value(&'a'));
        let mut decoder_for_b = huffman.decoder();
        assert_eq!(decoder_for_b.consume(1), DecodingResult::Value(&'b'));
        let mut decoder_for_c = huffman.decoder();
        assert_eq!(decoder_for_c.consume(2), DecodingResult::Value(&'c'));
        let mut decoder_for_invalid = huffman.decoder();
        assert_eq!(decoder_for_invalid.consume(3), DecodingResult::Invalid);
        test_read_write(&huffman);
    }

    #[test]
    fn coding_6sym_1bit() {
        //     /   \
        //   /  \  /\
        //  / \ d  ef
        // /\ a
        // bc
        let frequencies = hashmap!('d' => 12, 'e' => 11, 'f' => 10, 'a' => 3, 'b' => 2, 'c' => 1);
        let huffman = Coding::from_frequencies(BitsPerFragment(1), frequencies);
        assert_eq!(huffman.total_fragments_count(), 17);
        assert_eq!(huffman.values.as_ref(), ['d', 'e', 'f', 'a', 'b', 'c']);
        assert_eq!(huffman.internal_nodes_count.as_ref(), [2, 1, 1, 0]);
        assert_eq!(huffman.codes_for_values(), hashmap!(
                'a' => Code{bits: 0b001, fragments: 3 },
                'b' => Code{bits: 0b0000, fragments: 4 },
                'c' => Code{bits: 0b0001, fragments: 4 },
                'd' => Code{bits: 0b01, fragments: 2 },
                'e' => Code{bits: 0b10, fragments: 2 },
                'f' => Code{bits: 0b11, fragments: 2 }
               ));
        let mut decoder_for_a = huffman.decoder();
        assert_eq!(decoder_for_a.consume(0), DecodingResult::Incomplete);
        assert_eq!(decoder_for_a.consume(0), DecodingResult::Incomplete);
        assert_eq!(decoder_for_a.consume(1), DecodingResult::Value(&'a'));
        let mut decoder_for_b = huffman.decoder();
        assert_eq!(decoder_for_b.consume(0), DecodingResult::Incomplete);
        assert_eq!(decoder_for_b.consume(0), DecodingResult::Incomplete);
        assert_eq!(decoder_for_b.consume(0), DecodingResult::Incomplete);
        assert_eq!(decoder_for_b.consume(0), DecodingResult::Value(&'b'));
        let mut decoder_for_c = huffman.decoder();
        assert_eq!(decoder_for_c.consume(0), DecodingResult::Incomplete);
        assert_eq!(decoder_for_c.consume(0), DecodingResult::Incomplete);
        assert_eq!(decoder_for_c.consume(0), DecodingResult::Incomplete);
        assert_eq!(decoder_for_c.consume(1), DecodingResult::Value(&'c'));
        let mut decoder_for_d = huffman.decoder();
        assert_eq!(decoder_for_d.consume(0), DecodingResult::Incomplete);
        assert_eq!(decoder_for_d.consume(1), DecodingResult::Value(&'d'));
        let mut decoder_for_e = huffman.decoder();
        assert_eq!(decoder_for_e.consume(1), DecodingResult::Incomplete);
        assert_eq!(decoder_for_e.consume(0), DecodingResult::Value(&'e'));
        let mut decoder_for_f = huffman.decoder();
        assert_eq!(decoder_for_f.consume(1), DecodingResult::Incomplete);
        assert_eq!(decoder_for_f.consume(1), DecodingResult::Value(&'f'));
        test_read_write(&huffman);
    }

    #[test]
    fn coding_6sym_2bits() {
        //  /   |  \  \
        // /\\  d  e  f
        // abc 12 11 10
        // 321
        let frequencies = hashmap!('d' => 12, 'e' => 11, 'f' => 10, 'a' => 3, 'b' => 2, 'c' => 1);
        let huffman = Coding::from_frequencies(BitsPerFragment(2), frequencies);
        assert_eq!(huffman.total_fragments_count(), 9);
        assert_eq!(huffman.values.as_ref(), ['d', 'e', 'f', 'a', 'b', 'c']);
        assert_eq!(huffman.internal_nodes_count.as_ref(), [1, 0]);
        assert_eq!(huffman.codes_for_values(), hashmap!(
                'a' => Code{bits: 0b00_00, fragments: 2 },
                'b' => Code{bits: 0b00_01, fragments: 2 },
                'c' => Code{bits: 0b00_10, fragments: 2 },
                'd' => Code{bits: 0b01, fragments: 1 },
                'e' => Code{bits: 0b10, fragments: 1 },
                'f' => Code{bits: 0b11, fragments: 1 }
               ));
        let mut decoder_for_a = huffman.decoder();
        assert_eq!(decoder_for_a.consume(0), DecodingResult::Incomplete);
        assert_eq!(decoder_for_a.consume(0), DecodingResult::Value(&'a'));
        let mut decoder_for_b = huffman.decoder();
        assert_eq!(decoder_for_b.consume(0), DecodingResult::Incomplete);
        assert_eq!(decoder_for_b.consume(1), DecodingResult::Value(&'b'));
        let mut decoder_for_c = huffman.decoder();
        assert_eq!(decoder_for_c.consume(0), DecodingResult::Incomplete);
        assert_eq!(decoder_for_c.consume(2), DecodingResult::Value(&'c'));
        let mut decoder_for_invalid = huffman.decoder();
        assert_eq!(decoder_for_invalid.consume(0), DecodingResult::Incomplete);
        assert_eq!(decoder_for_invalid.consume(3), DecodingResult::Invalid);
        let mut decoder_for_d = huffman.decoder();
        assert_eq!(decoder_for_d.consume(1), DecodingResult::Value(&'d'));
        let mut decoder_for_e = huffman.decoder();
        assert_eq!(decoder_for_e.consume(2), DecodingResult::Value(&'e'));
        let mut decoder_for_f = huffman.decoder();
        assert_eq!(decoder_for_f.consume(3), DecodingResult::Value(&'f'));
        test_read_write(&huffman);
    }

    #[test]
    fn coding_5sym_tree_degree3() {
        //  /   |  \
        // /\\  d  e
        // abc 12 11
        // 321
        let frequencies = hashmap!('d' => 12, 'e' => 11, 'a' => 3, 'b' => 2, 'c' => 1);
        let huffman = Coding::from_frequencies(Degree(3), frequencies);
        assert_eq!(huffman.total_fragments_count(), 8);
        assert_eq!(huffman.values.as_ref(), ['d', 'e', 'a', 'b', 'c']);
        assert_eq!(huffman.internal_nodes_count.as_ref(), [1, 0]);
        assert_eq!(huffman.codes_for_values(), hashmap!(
                'a' => Code{bits: 0+0, fragments: 2 },
                'b' => Code{bits: 0+1, fragments: 2 },
                'c' => Code{bits: 0+2, fragments: 2 },
                'd' => Code{bits: 1, fragments: 1 },
                'e' => Code{bits: 2, fragments: 1 }
               ));
        let mut decoder_for_a = huffman.decoder();
        assert_eq!(decoder_for_a.consume(0), DecodingResult::Incomplete);
        assert_eq!(decoder_for_a.consume(0), DecodingResult::Value(&'a'));
        let mut decoder_for_b = huffman.decoder();
        assert_eq!(decoder_for_b.consume(0), DecodingResult::Incomplete);
        assert_eq!(decoder_for_b.consume(1), DecodingResult::Value(&'b'));
        let mut decoder_for_c = huffman.decoder();
        assert_eq!(decoder_for_c.consume(0), DecodingResult::Incomplete);
        assert_eq!(decoder_for_c.consume(2), DecodingResult::Value(&'c'));
        let mut decoder_for_invalid = huffman.decoder();
        assert_eq!(decoder_for_invalid.consume(0), DecodingResult::Incomplete);
        assert_eq!(decoder_for_invalid.consume(3), DecodingResult::Invalid);
        let mut decoder_for_d = huffman.decoder();
        assert_eq!(decoder_for_d.consume(1), DecodingResult::Value(&'d'));
        let mut decoder_for_e = huffman.decoder();
        assert_eq!(decoder_for_e.consume(2), DecodingResult::Value(&'e'));
        test_read_write(&huffman);
    }
}