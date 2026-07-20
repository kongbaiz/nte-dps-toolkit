.code

; Minimal no-CRT runtime support used by compiler-generated aggregate clears.
; Keep this file independent from the generated dwmapi forwarding stubs.
public memset
memset proc
  mov rax, rcx
  test r8, r8
  jz memset_done
memset_loop:
  mov byte ptr [rcx], dl
  inc rcx
  dec r8
  jnz memset_loop
memset_done:
  ret
memset endp

end
