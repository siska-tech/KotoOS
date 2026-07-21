# Fetch Weather sample

This KotoSDK sample starts one allowlisted HTTPS `GET`, yields while pending,
reads the response through a caller-owned 512-byte buffer, and renders fixed
offline/denied/failure states without blocking the frame loop. KotoSim supplies
a deterministic JSON response and never uses the host network or wall clock.
