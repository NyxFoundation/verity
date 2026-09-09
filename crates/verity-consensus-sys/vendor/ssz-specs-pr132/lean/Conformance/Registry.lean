import Conformance.Reader

/-! The types the reference tests are written over. -/

namespace Conformance

open Ssz

/-- A boolean. -/
def boolean : Desc := .bool

/--
An unsigned integer of the given width in bytes.

Prefer the named widths below, since a byte count reads as the wrong type:

A width of 8 is a sixty-four bit integer, not an eight bit one.
-/
def uint (width : Nat) : Desc := .uint width

/-- The six unsigned integers SSZ defines. -/
def uint8 : Desc := Desc.uint8

@[inherit_doc uint8] def uint16 : Desc := Desc.uint16

@[inherit_doc uint8] def uint32 : Desc := Desc.uint32

@[inherit_doc uint8] def uint64 : Desc := Desc.uint64

@[inherit_doc uint8] def uint128 : Desc := Desc.uint128

@[inherit_doc uint8] def uint256 : Desc := Desc.uint256

/-- A fixed number of opaque bytes. -/
def byteVector (length : Nat) : Desc := .byteVector length

/-- A variable number of opaque bytes, up to a limit. -/
def byteList (limit : Nat) : Desc := .byteList limit

/-- A fixed number of bits. -/
def bitVector (length : Nat) : Desc := .bitVector length

/-- A variable number of bits, up to a limit. -/
def bitList (limit : Nat) : Desc := .bitList limit

/-- A variable number of bits with no limit. -/
def progressiveBitList : Desc := .progressiveBitList

/-- A fixed number of elements. -/
def vector (element : Desc) (length : Nat) : Desc := .vector element length

/-- A variable number of elements, up to a limit. -/
def list (element : Desc) (limit : Nat) : Desc := .list element limit

/-- A variable number of elements with no limit. -/
def progressiveList (element : Desc) : Desc := .progressiveList element

/-- A fixed sequence of named fields. -/
def container (fields : List (String × Desc)) : Desc :=
  -- Names and field types are projected in the same order, preserving their pairing.
  .container (fields.map fun (name, _) => name) (fields.map fun (_, field) => field)

/-- Named fields that keep their tree positions as the set of them changes. -/
def progressiveContainer (active : List Bool) (fields : List (String × Desc)) : Desc :=
  -- Field order stays independent of the active mask’s gaps.
  .progressiveContainer active (fields.map fun (name, _) => name)
    (fields.map fun (_, field) => field)

/-- A choice between options that share one tree shape. -/
def compatibleUnion (options : List (Nat × Desc)) : Desc :=
  -- Selectors and option types retain the same ordinal when the paired declaration is split.
  .compatibleUnion (options.map fun (selector, _) => selector)
    (options.map fun (_, option) => option)

/-- Thirty-two opaque bytes, used as an element type by several fixtures. -/
def bytes32 : Desc := byteVector 32

/-- A square, occupying the first and third positions of a three-wide layout. -/
def sampleSquare : Desc :=
  progressiveContainer [true, false, true] [("side", uint16), ("color", uint8)]

/-- A circle, occupying the second and third positions of the same layout. -/
def sampleCircle : Desc :=
  progressiveContainer [false, true, true] [("radius", uint16), ("color", uint8)]

/-- A short list of small integers, used as a union option. -/
def sampleUint16List4 : Desc := list uint16 4

/-- A union over two shapes that share one tree. -/
def sampleShape : Desc :=
  compatibleUnion [(1, sampleSquare), (2, sampleCircle), (127, sampleSquare)]

/-- A union over one option, used to show that a lone option still mixes its selector in. -/
def sampleSquareOnly : Desc := compatibleUnion [(5, sampleSquare)]

/-- A struct nested inside another, to show a position holding a whole subtree. -/
def sampleInnerShape : Desc :=
  progressiveContainer [true, false, true] [("x", uint16), ("y", uint8)]

/-- An unbounded list of small integers, used as a nested element type. -/
def sampleUint16ProgressiveList : Desc := progressiveList uint16

/-- An unbounded list of large integers, used as a field and on its own. -/
def sampleUint64ProgressiveList : Desc := progressiveList uint64

/--
Every type the reference tests name, keyed by the module that declares it.

Two modules declare a struct under one name with different fields.
The module is therefore part of the key, not the name alone.
-/
def registry : List (String × Desc) := [
  -- Every basic shape, and the collections built directly from them.
  ("test_basic_types/Boolean", boolean),
  ("test_basic_types/Uint8", uint8),
  ("test_basic_types/Uint16", uint16),
  ("test_basic_types/Uint32", uint32),
  ("test_basic_types/Uint64", uint64),
  ("test_basic_types/Uint128", uint128),
  ("test_basic_types/Uint256", uint256),
  ("test_basic_types/Bytes4", byteVector 4),
  ("test_basic_types/Bytes32", bytes32),
  ("test_basic_types/Bytes52", byteVector 52),
  ("test_basic_types/Bytes64", byteVector 64),
  ("test_basic_types/ByteList512KiB", byteList (512 * 1024)),
  ("test_basic_types/SampleBitVector8", bitVector 8),
  ("test_basic_types/SampleBitVector64", bitVector 64),
  ("test_basic_types/SampleBitList16", bitList 16),
  ("test_basic_types/SampleUint16Vector3", vector uint16 3),
  ("test_basic_types/SampleUint64Vector4", vector uint64 4),
  ("test_basic_types/SampleUint32List16", list uint32 16),
  ("test_basic_types/SampleBytes32List8", list bytes32 8),

  -- Widths that land on, just under, and just over a node boundary.
  ("test_merkleization_boundaries/BoundaryBitVector1", bitVector 1),
  ("test_merkleization_boundaries/BoundaryBitVector7", bitVector 7),
  ("test_merkleization_boundaries/BoundaryBitVector9", bitVector 9),
  ("test_merkleization_boundaries/BoundaryBitVector255", bitVector 255),
  ("test_merkleization_boundaries/BoundaryBitVector256", bitVector 256),
  ("test_merkleization_boundaries/BoundaryBitVector257", bitVector 257),
  ("test_merkleization_boundaries/BoundaryBitList256", bitList 256),
  ("test_merkleization_boundaries/BoundaryUint64List32", list uint64 32),

  -- The one shape whose fixtures record a refusal rather than a value.
  ("test_decode_failure_smoke/SmokeBitList8", bitList 8),

  -- The shapes whose trees grow with their data rather than to a declared capacity.
  ("test_progressive_types/Bytes32", bytes32),
  ("test_progressive_types/ProgressiveBitList", progressiveBitList),
  ("test_progressive_types/SampleUint64ProgressiveList", sampleUint64ProgressiveList),
  ("test_progressive_types/SampleBytes32ProgressiveList", progressiveList bytes32),
  ("test_progressive_types/SampleNestedProgressiveList",
    progressiveList sampleUint16ProgressiveList),
  ("test_progressive_types/SampleContainerWithProgressiveList",
    container [("a", uint16), ("b", sampleUint64ProgressiveList), ("c", uint8)]),

  -- Layouts that place their fields around gaps of differing widths.
  ("test_progressive_containers/SampleUint64ProgressiveList", sampleUint64ProgressiveList),
  ("test_progressive_containers/ProgressiveBitList", progressiveBitList),
  ("test_progressive_containers/SampleSquare", sampleSquare),
  ("test_progressive_containers/SampleCircle", sampleCircle),
  ("test_progressive_containers/SampleOneField",
    progressiveContainer [true] [("a", uint16)]),
  ("test_progressive_containers/SampleLeadingGaps",
    progressiveContainer [false, false, true] [("c", uint32)]),
  ("test_progressive_containers/SampleMultipleGaps",
    progressiveContainer [true, false, false, true, false, true]
      [("a", uint8), ("b", uint16), ("c", uint32)]),
  ("test_progressive_containers/SampleWidestLayout",
    progressiveContainer (List.replicate 255 false ++ [true]) [("tail", uint8)]),
  ("test_progressive_containers/SampleLevelBoundary",
    progressiveContainer ([true] ++ List.replicate 20 false ++ [true])
      [("first", uint16), ("last", uint8)]),
  ("test_progressive_containers/SampleBoundedListField",
    progressiveContainer [true, false, true]
      [("head", uint64), ("body", sampleUint16List4)]),
  ("test_progressive_containers/SampleProgressiveFields",
    progressiveContainer [true, true, true]
      [("head", uint64), ("numbers", sampleUint64ProgressiveList),
       ("flags", progressiveBitList)]),
  ("test_progressive_containers/SampleOuterShape",
    progressiveContainer [true, false, true]
      [("head", uint8), ("inner", sampleInnerShape)]),
  ("test_progressive_containers/SampleSquareProgressiveList", progressiveList sampleSquare),
  ("test_progressive_containers/SampleShapeContainer",
    container [("tag", uint8), ("shape", sampleSquare)]),

  -- Options that merkleize alike, which is what lets one tree serve them all.
  ("test_compatible_unions/SampleSquare", sampleSquare),
  ("test_compatible_unions/SampleCircle", sampleCircle),
  ("test_compatible_unions/SampleSquareProgressiveList", progressiveList sampleSquare),
  ("test_compatible_unions/SampleShape", sampleShape),
  ("test_compatible_unions/SampleNumbers",
    compatibleUnion [(1, sampleUint16List4), (2, sampleUint16List4)]),
  ("test_compatible_unions/SampleEmptyProne",
    compatibleUnion [(1, progressiveList sampleSquare), (2, progressiveList sampleCircle)]),
  ("test_compatible_unions/SampleNestedShape",
    compatibleUnion [(1, sampleShape), (2, sampleSquareOnly)]),
  ("test_compatible_unions/SampleShapeContainer",
    container [("tag", uint64), ("body", sampleShape)]),
  ("test_compatible_unions/SampleShapeProgressiveContainer",
    progressiveContainer [true, false, true] [("tag", uint64), ("body", sampleShape)]),
  ("test_compatible_unions/SampleShapeProgressiveList", progressiveList sampleShape)]

/-- The type a fixture names, looked up by its module and its name. -/
def lookupShape (moduleName typeName : String) : Option Desc :=
  -- The module prefix disambiguates fixture types that share a short name.
  (registry.find? fun (key, _) => key == moduleName ++ "/" ++ typeName).map
    fun (_, shape) => shape

end Conformance
