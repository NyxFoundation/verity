/-! Why a type, a value, an encoding, or a proof was refused. -/

namespace Ssz

/-- Bytes, as the codec passes them around. -/
abbrev Bytes := Array UInt8

/-- A refusal, naming what was broken. -/
inductive Err where
  /-- A value of the wrong shape for the type it was read against. -/
  | typeMismatch
  /-- A count above the capacity the type declares. -/
  | overLimit (limit actual : Nat)
  /-- A budget that is not the exact width the type encodes to. -/
  | scope (expected actual : Nat)
  /-- A budget too small to hold even the fixed part. -/
  | scopeTooSmall (expected actual : Nat)
  /-- A budget that is no whole number of elements. -/
  | scopeUndivided (scope width : Nat)
  /-- A budget spent on elements that encode to nothing. -/
  | scopeWidthless
  /-- A first offset that does not land on the end of the fixed part. -/
  | firstOffset (expected actual : Nat)
  /-- An offset above the one after it, which would overlap two bodies. -/
  | offsetUnordered
  /-- An offset past the end of the budget. -/
  | offsetPastScope
  /-- An offset that is no whole number of table entries. -/
  | offsetUnaligned
  /-- An offset pointing inside the table rather than past it. -/
  | offsetBelowTable
  /-- Fewer bytes than the budget promised. -/
  | truncated
  /-- Padding bits set above the last declared bit. -/
  | paddingBits
  /-- An empty encoding, where even an empty value takes a byte. -/
  | emptyEncoding
  /-- A bitlist encoding with no closing bit anywhere in it. -/
  | noDelimiter
  /-- Zero bytes past the closing bit, which would give one value two encodings. -/
  | trailingZeros
  /-- A union encoding too short to hold its selector. -/
  | noSelector
  /-- A selector naming no option of the union. -/
  | unknownSelector (selector : Nat)
  /-- A type whose parts do not pair up, which names no shape at all. -/
  | badDeclaration
  /-- A number naming no node of any tree. -/
  | notAGindex (index : Nat)
  /-- The root, which sits on no branch of its own. -/
  | rootHasNoBranch
  /-- A request naming no index, which would check nothing. -/
  | emptyRequest
  /-- One index named twice, which would keep one value and drop the other. -/
  | repeatedIndex
  /-- An index below another in the same request, which the other would rebuild. -/
  | nestedIndex (index : Nat)
  /-- A branch whose length is not the depth of the index it authenticates. -/
  | branchLength (expected actual : Nat)
  /-- Leaves that do not pair one to one with the indices claimed. -/
  | leafCount (expected actual : Nat)
  /-- A proof carrying a different number of nodes than the request needs. -/
  | proofLength (expected actual : Nat)
  /-- A proof from which no root could be rebuilt. -/
  | proofIncomplete
  /-- A path descending into a mixed-in word, which has no parts. -/
  | pathIntoMixin
  /-- A path descending into packed data, where no element has a node of its own. -/
  | pathIntoPacked
  /-- A path descending into a layout position that holds no field. -/
  | pathIntoGap
  /-- A path onto a level past the end of a progressive spine. -/
  | pathPastSpine
  /-- A step into a basic value, which has no parts to address. -/
  | noParts
  /-- A step past a mixed-in word, which likewise has no parts. -/
  | noPartsMixin
  /-- A reserved step naming a word the type does not mix in. -/
  | noMixin
  /-- A leaf count asked of a shape that grows without bound. -/
  | noChunkCount
  /-- A step into a shape that cannot be stepped into. -/
  | notSteppable
  /-- A field position the struct does not declare. -/
  | noSuchField (step : Nat)
  /-- A selector the union does not declare. -/
  | noSuchOption (step : Nat)
  /-- A position the shape does not declare. -/
  | noSuchPosition (step : Nat)
  /-- More chunks than the merkleization capacity allows. -/
  | merkleizeLimit (count limit : Nat)
  /-- A zero subtree asked for at a width that is no power of two, or past the table. -/
  | zeroTreeWidth (width : Nat)
  /-- A vector of no elements, where a vector holds at least one. -/
  | vectorEmpty
  /-- A field layout of no positions, where a layout holds at least one. -/
  | layoutWidth
  /-- A field layout whose last position is a gap rather than a field. -/
  | layoutTrailingGap
  /-- A field layout wider than the limit. -/
  | layoutTooWide (width limit : Nat)
  /-- A field layout setting a different number of positions than the struct has fields. -/
  | layoutFieldCount (active declared : Nat)
  /-- A union declaring no option, where a union declares at least one. -/
  | unionEmpty
  /-- A selector outside the range a union may use. -/
  | unionSelectorRange (selector low high : Nat)
  /-- Two options of one union that merkleize differently. -/
  | unionIncompatible (selector other : Nat)
  /-- A union repeating one selector. -/
  | unionSelectorRepeated (selector : Nat)
  /-- An unsigned integer of a width SSZ does not define. -/
  | uintWidth (width : Nat)
  /-- An encoding whose parts run past what an offset can name. -/
  | offsetOverflow (offset : Nat)
  /-- A struct with no fields, whose encoding would be empty at any value. -/
  | containerEmpty
  /-- A fixed-width shape declaring no positions, which encodes to nothing. -/
  | widthZero
  deriving Repr, BEq, DecidableEq

/--
The name the specification records for a refusal.

The reference tests carry this name for every encoding they expect to be refused.
A refusal is then checked for its reason, and not merely for happening.
-/
def Err.reason : Err → String
  -- Diagnostic names distinguish malformed input, invalid declarations, and unreadable proof requests.
  | .typeMismatch => "WRONG_TYPE"
  | .overLimit _ _ => "LIMIT"
  | .scope _ _ => "SCOPE"
  | .scopeTooSmall _ _ => "SCOPE_TOO_SMALL"
  | .scopeUndivided _ _ => "SCOPE_UNDIVIDED"
  | .scopeWidthless => "SCOPE_WIDTHLESS"
  | .firstOffset _ _ => "FIRST_OFFSET"
  | .offsetUnordered => "OFFSET_UNORDERED"
  | .offsetPastScope => "OFFSET_PAST_SCOPE"
  | .offsetUnaligned => "OFFSET_UNALIGNED"
  | .offsetBelowTable => "OFFSET_BELOW_TABLE"
  | .truncated => "TRUNCATED"
  | .paddingBits => "PADDING_BITS"
  | .emptyEncoding => "EMPTY_ENCODING"
  | .noDelimiter => "NO_DELIMITER"
  | .trailingZeros => "TRAILING_ZEROS"
  | .noSelector => "NO_SELECTOR"
  | .unknownSelector _ => "UNKNOWN_SELECTOR"
  | .notAGindex _ => "NOT_A_GINDEX"
  | .rootHasNoBranch => "ROOT_HAS_NO_BRANCH"
  | .emptyRequest => "EMPTY_REQUEST"
  | .repeatedIndex => "REPEATED_INDEX"
  | .nestedIndex _ => "NESTED_INDEX"
  | .branchLength _ _ => "BRANCH_LENGTH"
  | .leafCount _ _ => "LEAF_COUNT"
  | .proofLength _ _ => "PROOF_LENGTH"
  | .pathIntoMixin => "PATH_INTO_MIXIN"
  | .pathIntoPacked => "PATH_INTO_PACKED"
  | .pathIntoGap => "PATH_INTO_GAP"
  | .pathPastSpine => "PATH_PAST_SPINE"
  | .noParts => "NO_PARTS"
  | .noPartsMixin => "NO_PARTS_MIXIN"
  | .noMixin => "NO_MIXIN"
  | .noChunkCount => "NO_CHUNK_COUNT"
  | .notSteppable => "NOT_STEPPABLE"
  | .noSuchField _ => "NO_SUCH_FIELD"
  | .noSuchOption _ => "NO_SUCH_OPTION"
  | .noSuchPosition _ => "NO_SUCH_POSITION"
  | .merkleizeLimit _ _ => "MERKLEIZE_LIMIT"
  | .zeroTreeWidth _ => "ZERO_TREE_WIDTH"
  | .vectorEmpty => "VECTOR_EMPTY"
  | .layoutWidth => "LAYOUT_WIDTH"
  | .layoutTrailingGap => "LAYOUT_TRAILING_GAP"
  | .layoutTooWide _ _ => "LAYOUT_TOO_WIDE"
  | .layoutFieldCount _ _ => "LAYOUT_FIELD_COUNT"
  | .unionEmpty => "UNION_EMPTY"
  | .unionSelectorRange _ _ _ => "UNION_SELECTOR_RANGE"
  | .unionIncompatible _ _ => "UNION_INCOMPATIBLE"
  | .unionSelectorRepeated _ => "UNION_SELECTOR_REPEATED"
  | .uintWidth _ => "UINT_WIDTH"
  | .offsetOverflow _ => "OFFSET_OVERFLOW"
  | .containerEmpty => "CONTAINER_EMPTY"
  | .widthZero => "WIDTH_ZERO"
  | .proofIncomplete => "PROOF_INCOMPLETE"
  | .badDeclaration => "BAD_DECLARATION"

end Ssz
