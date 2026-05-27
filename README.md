# Verity

The Provable Consensus Client — a formally verified Ethereum consensus client built with Lean by [Nyx Foundation](https://github.com/NyxFoundation).

> Verity is currently under active development and has not been released yet. Stay tuned for updates.

## What is Verity?

Where other clients test for correctness, Verity proves it. Every line of consensus logic is mathematically proven correct, closing the gap between specification and implementation — permanently.

Learn more at [verityclient.com](https://verityclient.com).

## Acknowledgments

Verity does not start from zero. It builds on a young, fast-moving ecosystem of Lean Consensus implementations whose authors did the hard work of turning a moving specification into running clients — and Verity has learned from every one of them.

- **[ethlambda](https://github.com/lambdaclass/ethlambda)** by LambdaClass is a study in disciplined minimalism: a clean-slate, consensus-only client that carries post-quantum, hash-based signatures into the production path from day one and refuses to accumulate the complexity Lean Ethereum was designed to shed. Keeping the surface that small is exactly the spirit a tractable proof needs.
- **[ream](https://github.com/ReamLabs/ream)** by ReamLabs sets a high bar for engineering breadth — a Beacon/Lean dual stack, zkVM-friendly cryptography, serious networking, and a tooling and CI discipline worth imitating. Its habit of expressing collection bounds in the type, rather than checking them at runtime, is precisely the kind of invariant Verity wants inside its verified core.

Verity makes a different bet — that the implementation should be *proven* to match the specification, not only tested against it — but that bet is worth making only because these clients have already shown what a good Lean Consensus client looks like. We are grateful to their authors and to the wider [Lean Ethereum](https://leanroadmap.org/) community.

## References

- https://github.com/leanEthereum/leanSpec
- https://github.com/leanEthereum/leanMultisig
- https://github.com/leanEthereum/leanMetrics
- https://hive.leanroadmap.org/
- https://observatory.leanroadmap.org/
- https://leanroadmap.org/
- https://strawmap.org/

## License

[MIT](./LICENSE)
