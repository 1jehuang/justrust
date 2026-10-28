import Lean

/-
Bounded model, NOT a proof of the Rust compiler. A header has one input and
one output type. Named types resolve directly to numeric semantic identities.
No aliases, cycles, generics, body inference, errors or Rust parsing are modeled.
Every modeled read returns both its value and its dependency trace. There is
no arbitrary callback or world access in the scoped producer below.
-/
namespace DependencyPilot

inductive TypeExpr where
  | scalar : Nat → TypeExpr
  | named : Nat → TypeExpr
  deriving DecidableEq, Repr

structure Header where
  input : TypeExpr
  output : TypeExpr
  deriving DecidableEq, Repr

inductive Key where
  | header : Nat → Key
  | typeDecl : Nat → Key
  | body : Nat → Key
  | allSignatures : Key
  deriving DecidableEq, Repr

structure World where
  headers : Nat → Header
  types : Nat → Nat
  bodies : Nat → Nat
  globalSignatures : Nat

-- The scoped worker cannot receive the world or request bodies/global tables.
inductive Request where
  | typeDecl : Nat → Request
  deriving DecidableEq, Repr

def Request.key : Request → Key
  | .typeDecl n => .typeDecl n

def resolve (lookup : Nat → Nat) : TypeExpr → Nat × List Request
  | .scalar n => (n, [])
  | .named n => (lookup n, [.typeDecl n])

def signature (h : Header) (lookup : Nat → Nat) : (Nat × Nat) × List Request :=
  let a := resolve lookup h.input
  let b := resolve lookup h.output
  ((a.1, b.1), a.2 ++ b.2)

-- This is the modeled graph adapter. Its world reads are explicit here.
def run (w : World) (owner : Nat) : (Nat × Nat) × List Key :=
  let answer := signature (w.headers owner) w.types
  (answer.1, .header owner :: answer.2.map Request.key)

def Allowed (owner : Nat) (k : Key) : Prop :=
  k = .header owner ∨ ∃ n, k = .typeDecl n

theorem resolve_only_types (lookup : Nat → Nat) (t : TypeExpr)
    (r : Request) (hr : r ∈ (resolve lookup t).2) :
    ∃ n, r.key = .typeDecl n := by
  cases t <;> simp [resolve] at hr
  subst r
  exact ⟨_, rfl⟩

theorem scoped_requests (w : World) (owner : Nat) :
    ∀ k ∈ (run w owner).2, Allowed owner k := by
  intro k hk
  simp only [run, signature, List.map_append, List.mem_cons,
    List.mem_append, List.mem_map] at hk
  rcases hk with h | h | h
  · exact Or.inl h
  · obtain ⟨r, hr, rfl⟩ := h
    exact Or.inr (resolve_only_types _ _ _ hr)
  · obtain ⟨r, hr, rfl⟩ := h
    exact Or.inr (resolve_only_types _ _ _ hr)

def typeDependencies : TypeExpr → List Key
  | .scalar _ => []
  | .named n => [.typeDecl n]

theorem resolve_exact_dependencies (lookup : Nat → Nat) (t : TypeExpr) :
    (resolve lookup t).2.map Request.key = typeDependencies t := by
  cases t <;> rfl

theorem exact_dependencies (w : World) (owner : Nat) :
    (run w owner).2 = .header owner ::
      (typeDependencies (w.headers owner).input ++
       typeDependencies (w.headers owner).output) := by
  simp [run, signature, List.map_append, resolve_exact_dependencies]

-- Agreement is needed only on the references actually present in this header.
def AgreesOnType (a b : Nat → Nat) : TypeExpr → Prop
  | .scalar _ => True
  | .named n => a n = b n

theorem resolve_sufficient (a b : Nat → Nat) (t : TypeExpr)
    (h : AgreesOnType a b t) : resolve a t = resolve b t := by
  cases t <;> simp_all [AgreesOnType, resolve]

theorem signature_sufficient (h : Header) (a b : Nat → Nat)
    (hi : AgreesOnType a b h.input) (ho : AgreesOnType a b h.output) :
    signature h a = signature h b := by
  simp only [signature, resolve_sufficient a b h.input hi,
    resolve_sufficient a b h.output ho]

-- Arbitrary body and global-table replacements leave both answer AND trace equal.
theorem unrelated_bodies (w : World) (owner : Nat)
    (bodies : Nat → Nat) (global : Nat) :
    run { w with bodies := bodies, globalSignatures := global } owner =
      run w owner := by
  rfl

-- A signature with one named input. Its two distinct fact dependencies really
-- are necessary across this model's domain, not just an upper-bound read set.
def exampleHeader : Header := ⟨.named 7, .scalar 0⟩
def base : World := ⟨fun _ => exampleHeader, fun _ => 0, fun _ => 0, 0⟩
def changedType : World := { base with types := fun n => if n = 7 then 1 else 0 }
def changedHeader : World :=
  { base with headers := fun n => if n = 3 then ⟨.scalar 2, .scalar 0⟩ else exampleHeader }

theorem example_exact_dependencies :
    (run base 3).2 = [.header 3, .typeDecl 7] := by rfl

theorem type_fact_necessary :
    base.headers = changedType.headers ∧
    (∀ n, n ≠ 7 → base.types n = changedType.types n) ∧
    (run base 3).1 ≠ (run changedType 3).1 := by
  constructor
  · rfl
  constructor
  · intro n hn
    simp [base, changedType, hn]
  · decide

theorem header_fact_necessary :
    base.types = changedHeader.types ∧
    (∀ n, n ≠ 3 → base.headers n = changedHeader.headers n) ∧
    (run base 3).1 ≠ (run changedHeader 3).1 := by
  constructor
  · rfl
  constructor
  · intro n hn
    simp [base, changedHeader, hn]
  · decide

-- Semantically redundant reads can leave the answer correct. The access
-- contract must reject them separately rather than confusing result equality
-- with minimal dependencies.
def broadRun (w : World) (owner : Nat) : (Nat × Nat) × List Key :=
  let result := run w owner
  (result.1, .allSignatures :: result.2)

theorem broad_answer_unchanged (w : World) (owner : Nat) :
    (broadRun w owner).1 = (run w owner).1 := by rfl

theorem broad_request_rejected (w : World) (owner : Nat) :
    ¬ (∀ k ∈ (broadRun w owner).2, Allowed owner k) := by
  intro h
  have bad := h .allSignatures (by simp [broadRun])
  simp [Allowed] at bad

#print axioms scoped_requests
#print axioms exact_dependencies
#print axioms signature_sufficient
#print axioms unrelated_bodies
#print axioms example_exact_dependencies
#print axioms type_fact_necessary
#print axioms header_fact_necessary
#print axioms broad_request_rejected

end DependencyPilot
