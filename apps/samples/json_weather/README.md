# JSON Weather sample

This KotoSDK sample feeds a fetched response through the bounded incremental
JSON decoder (`json_*`, KOTO-0246) chunk by chunk, without building a document
tree. It selects the named fields `location` and `temperature_c`, skips the
unknown nested `station` object and its array by nesting depth, and keeps
missing, duplicate, and wrong-type fields distinguishable. Each frame parses at
most one bounded chunk, so decoding never starves the frame loop. KotoSim
supplies a deterministic JSON response and never uses the host network.
