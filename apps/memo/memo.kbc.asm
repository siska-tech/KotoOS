; dev.koto.memo smoke program (KBC1 assembly / IR).
;
; This is the reproducible low-level source for sdcard_mock/bytecode/memo.kbc.
; It is the interim authoring form: the durable interactive memo app is written in
; the high-level Koto app language (KOTO-0045/0046) and compiled down to this layer.
; Regenerate the committed bytecode with:
;   cargo run -p kbc-asm -- apps/memo/memo.kbc.asm sdcard_mock/bytecode/memo.kbc
;
; Behavior: draw a banner, write "koto memo\n" to the sandboxed memo.txt, exit.

.stack 8
.calls 4
.heap 1024
.abi 1 0

; Materialize string data into the app heap.
store_str 0, "memo.txt"
store_str 16, "koto memo\n"

; draw_text(x=0, y=0, ptr=16, len=10)
push_i16 0
push_i16 0
push_i16 16
push_i16 10
host_call draw_text
drop

; handle = file_open(ptr=0, len=8, mode=1 write)
push_i16 0
push_i16 8
push_i16 1
host_call file_open
drop
store_local 0

; file_write(handle, ptr=16, len=10)
load_local 0
push_i16 16
push_i16 10
host_call file_write
drop
drop

; file_close(handle)
load_local 0
host_call file_close
drop

; exit(0)
push_i16 0
host_call exit
