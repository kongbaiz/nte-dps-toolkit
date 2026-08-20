.code
extern mProcs:QWORD
extern ResolveDwmapiExport:PROC

; Preserve the complete Windows x64 register argument set while the first call
; resolves the real system export outside DllMain. The original stack (and all
; stack arguments) is restored before the tail jump.
DwmapiDispatch proc
  lea r11, mProcs
  mov r11, [r11+r10*8]
  test r11, r11
  jz DwmapiResolve
  jmp r11
DwmapiDispatch endp

DwmapiResolve proc frame
  sub rsp, 88h
  .allocstack 88h
  .endprolog
  mov [rsp+20h], rcx
  mov [rsp+28h], rdx
  mov [rsp+30h], r8
  mov [rsp+38h], r9
  movdqu xmmword ptr [rsp+40h], xmm0
  movdqu xmmword ptr [rsp+50h], xmm1
  movdqu xmmword ptr [rsp+60h], xmm2
  movdqu xmmword ptr [rsp+70h], xmm3
  mov [rsp+80h], r10
  mov ecx, r10d
  call ResolveDwmapiExport
  mov rcx, [rsp+20h]
  mov rdx, [rsp+28h]
  mov r8, [rsp+30h]
  mov r9, [rsp+38h]
  movdqu xmm0, xmmword ptr [rsp+40h]
  movdqu xmm1, xmmword ptr [rsp+50h]
  movdqu xmm2, xmmword ptr [rsp+60h]
  movdqu xmm3, xmmword ptr [rsp+70h]
  mov r10, [rsp+80h]
  lea r11, mProcs
  add rsp, 88h
  jmp qword ptr [r11+r10*8]
DwmapiResolve endp
f0 proc
  mov r10d, 0
  jmp DwmapiDispatch
f0 endp
f1 proc
  mov r10d, 1
  jmp DwmapiDispatch
f1 endp
f2 proc
  mov r10d, 2
  jmp DwmapiDispatch
f2 endp
f3 proc
  mov r10d, 3
  jmp DwmapiDispatch
f3 endp
f4 proc
  mov r10d, 4
  jmp DwmapiDispatch
f4 endp
f5 proc
  mov r10d, 5
  jmp DwmapiDispatch
f5 endp
f6 proc
  mov r10d, 6
  jmp DwmapiDispatch
f6 endp
f7 proc
  mov r10d, 7
  jmp DwmapiDispatch
f7 endp
f8 proc
  mov r10d, 8
  jmp DwmapiDispatch
f8 endp
f9 proc
  mov r10d, 9
  jmp DwmapiDispatch
f9 endp
f10 proc
  mov r10d, 10
  jmp DwmapiDispatch
f10 endp
f11 proc
  mov r10d, 11
  jmp DwmapiDispatch
f11 endp
f12 proc
  mov r10d, 12
  jmp DwmapiDispatch
f12 endp
f13 proc
  mov r10d, 13
  jmp DwmapiDispatch
f13 endp
f14 proc
  mov r10d, 14
  jmp DwmapiDispatch
f14 endp
f15 proc
  mov r10d, 15
  jmp DwmapiDispatch
f15 endp
f16 proc
  mov r10d, 16
  jmp DwmapiDispatch
f16 endp
f17 proc
  mov r10d, 17
  jmp DwmapiDispatch
f17 endp
f18 proc
  mov r10d, 18
  jmp DwmapiDispatch
f18 endp
f19 proc
  mov r10d, 19
  jmp DwmapiDispatch
f19 endp
f20 proc
  mov r10d, 20
  jmp DwmapiDispatch
f20 endp
f21 proc
  mov r10d, 21
  jmp DwmapiDispatch
f21 endp
f22 proc
  mov r10d, 22
  jmp DwmapiDispatch
f22 endp
f23 proc
  mov r10d, 23
  jmp DwmapiDispatch
f23 endp
f24 proc
  mov r10d, 24
  jmp DwmapiDispatch
f24 endp
f25 proc
  mov r10d, 25
  jmp DwmapiDispatch
f25 endp
f26 proc
  mov r10d, 26
  jmp DwmapiDispatch
f26 endp
f27 proc
  mov r10d, 27
  jmp DwmapiDispatch
f27 endp
f28 proc
  mov r10d, 28
  jmp DwmapiDispatch
f28 endp
f29 proc
  mov r10d, 29
  jmp DwmapiDispatch
f29 endp
f30 proc
  mov r10d, 30
  jmp DwmapiDispatch
f30 endp
f31 proc
  mov r10d, 31
  jmp DwmapiDispatch
f31 endp
f32 proc
  mov r10d, 32
  jmp DwmapiDispatch
f32 endp
f33 proc
  mov r10d, 33
  jmp DwmapiDispatch
f33 endp
f34 proc
  mov r10d, 34
  jmp DwmapiDispatch
f34 endp
f35 proc
  mov r10d, 35
  jmp DwmapiDispatch
f35 endp
f36 proc
  mov r10d, 36
  jmp DwmapiDispatch
f36 endp
f37 proc
  mov r10d, 37
  jmp DwmapiDispatch
f37 endp
f38 proc
  mov r10d, 38
  jmp DwmapiDispatch
f38 endp
f39 proc
  mov r10d, 39
  jmp DwmapiDispatch
f39 endp
f40 proc
  mov r10d, 40
  jmp DwmapiDispatch
f40 endp
f41 proc
  mov r10d, 41
  jmp DwmapiDispatch
f41 endp
f42 proc
  mov r10d, 42
  jmp DwmapiDispatch
f42 endp
f43 proc
  mov r10d, 43
  jmp DwmapiDispatch
f43 endp
f44 proc
  mov r10d, 44
  jmp DwmapiDispatch
f44 endp
f45 proc
  mov r10d, 45
  jmp DwmapiDispatch
f45 endp
f46 proc
  mov r10d, 46
  jmp DwmapiDispatch
f46 endp
f47 proc
  mov r10d, 47
  jmp DwmapiDispatch
f47 endp
f48 proc
  mov r10d, 48
  jmp DwmapiDispatch
f48 endp
f49 proc
  mov r10d, 49
  jmp DwmapiDispatch
f49 endp
f50 proc
  mov r10d, 50
  jmp DwmapiDispatch
f50 endp
f51 proc
  mov r10d, 51
  jmp DwmapiDispatch
f51 endp
f52 proc
  mov r10d, 52
  jmp DwmapiDispatch
f52 endp
f53 proc
  mov r10d, 53
  jmp DwmapiDispatch
f53 endp
f54 proc
  mov r10d, 54
  jmp DwmapiDispatch
f54 endp
f55 proc
  mov r10d, 55
  jmp DwmapiDispatch
f55 endp
f56 proc
  mov r10d, 56
  jmp DwmapiDispatch
f56 endp
f57 proc
  mov r10d, 57
  jmp DwmapiDispatch
f57 endp
f58 proc
  mov r10d, 58
  jmp DwmapiDispatch
f58 endp
f59 proc
  mov r10d, 59
  jmp DwmapiDispatch
f59 endp
f60 proc
  mov r10d, 60
  jmp DwmapiDispatch
f60 endp
f61 proc
  mov r10d, 61
  jmp DwmapiDispatch
f61 endp
f62 proc
  mov r10d, 62
  jmp DwmapiDispatch
f62 endp
f63 proc
  mov r10d, 63
  jmp DwmapiDispatch
f63 endp
f64 proc
  mov r10d, 64
  jmp DwmapiDispatch
f64 endp
f65 proc
  mov r10d, 65
  jmp DwmapiDispatch
f65 endp
f66 proc
  mov r10d, 66
  jmp DwmapiDispatch
f66 endp
f67 proc
  mov r10d, 67
  jmp DwmapiDispatch
f67 endp
f68 proc
  mov r10d, 68
  jmp DwmapiDispatch
f68 endp
f69 proc
  mov r10d, 69
  jmp DwmapiDispatch
f69 endp
f70 proc
  mov r10d, 70
  jmp DwmapiDispatch
f70 endp
f71 proc
  mov r10d, 71
  jmp DwmapiDispatch
f71 endp
f72 proc
  mov r10d, 72
  jmp DwmapiDispatch
f72 endp
f73 proc
  mov r10d, 73
  jmp DwmapiDispatch
f73 endp
f74 proc
  mov r10d, 74
  jmp DwmapiDispatch
f74 endp
f75 proc
  mov r10d, 75
  jmp DwmapiDispatch
f75 endp
f76 proc
  mov r10d, 76
  jmp DwmapiDispatch
f76 endp
f77 proc
  mov r10d, 77
  jmp DwmapiDispatch
f77 endp
f78 proc
  mov r10d, 78
  jmp DwmapiDispatch
f78 endp
f79 proc
  mov r10d, 79
  jmp DwmapiDispatch
f79 endp
f80 proc
  mov r10d, 80
  jmp DwmapiDispatch
f80 endp
f81 proc
  mov r10d, 81
  jmp DwmapiDispatch
f81 endp
f82 proc
  mov r10d, 82
  jmp DwmapiDispatch
f82 endp
f83 proc
  mov r10d, 83
  jmp DwmapiDispatch
f83 endp
f84 proc
  mov r10d, 84
  jmp DwmapiDispatch
f84 endp
f85 proc
  mov r10d, 85
  jmp DwmapiDispatch
f85 endp
f86 proc
  mov r10d, 86
  jmp DwmapiDispatch
f86 endp
f87 proc
  mov r10d, 87
  jmp DwmapiDispatch
f87 endp
f88 proc
  mov r10d, 88
  jmp DwmapiDispatch
f88 endp
f89 proc
  mov r10d, 89
  jmp DwmapiDispatch
f89 endp
f90 proc
  mov r10d, 90
  jmp DwmapiDispatch
f90 endp
f91 proc
  mov r10d, 91
  jmp DwmapiDispatch
f91 endp
f92 proc
  mov r10d, 92
  jmp DwmapiDispatch
f92 endp
f93 proc
  mov r10d, 93
  jmp DwmapiDispatch
f93 endp
f94 proc
  mov r10d, 94
  jmp DwmapiDispatch
f94 endp
f95 proc
  mov r10d, 95
  jmp DwmapiDispatch
f95 endp
f96 proc
  mov r10d, 96
  jmp DwmapiDispatch
f96 endp
f97 proc
  mov r10d, 97
  jmp DwmapiDispatch
f97 endp
f98 proc
  mov r10d, 98
  jmp DwmapiDispatch
f98 endp
f99 proc
  mov r10d, 99
  jmp DwmapiDispatch
f99 endp
f100 proc
  mov r10d, 100
  jmp DwmapiDispatch
f100 endp
f101 proc
  mov r10d, 101
  jmp DwmapiDispatch
f101 endp
f102 proc
  mov r10d, 102
  jmp DwmapiDispatch
f102 endp
f103 proc
  mov r10d, 103
  jmp DwmapiDispatch
f103 endp
f104 proc
  mov r10d, 104
  jmp DwmapiDispatch
f104 endp
f105 proc
  mov r10d, 105
  jmp DwmapiDispatch
f105 endp
f106 proc
  mov r10d, 106
  jmp DwmapiDispatch
f106 endp
f107 proc
  mov r10d, 107
  jmp DwmapiDispatch
f107 endp
f108 proc
  mov r10d, 108
  jmp DwmapiDispatch
f108 endp
f109 proc
  mov r10d, 109
  jmp DwmapiDispatch
f109 endp
f110 proc
  mov r10d, 110
  jmp DwmapiDispatch
f110 endp
f111 proc
  mov r10d, 111
  jmp DwmapiDispatch
f111 endp
f112 proc
  mov r10d, 112
  jmp DwmapiDispatch
f112 endp
f113 proc
  mov r10d, 113
  jmp DwmapiDispatch
f113 endp
f114 proc
  mov r10d, 114
  jmp DwmapiDispatch
f114 endp
f115 proc
  mov r10d, 115
  jmp DwmapiDispatch
f115 endp
f116 proc
  mov r10d, 116
  jmp DwmapiDispatch
f116 endp
f117 proc
  mov r10d, 117
  jmp DwmapiDispatch
f117 endp
f118 proc
  mov r10d, 118
  jmp DwmapiDispatch
f118 endp
f119 proc
  mov r10d, 119
  jmp DwmapiDispatch
f119 endp
f120 proc
  mov r10d, 120
  jmp DwmapiDispatch
f120 endp
f121 proc
  mov r10d, 121
  jmp DwmapiDispatch
f121 endp
f122 proc
  mov r10d, 122
  jmp DwmapiDispatch
f122 endp
f123 proc
  mov r10d, 123
  jmp DwmapiDispatch
f123 endp
f124 proc
  mov r10d, 124
  jmp DwmapiDispatch
f124 endp
f125 proc
  mov r10d, 125
  jmp DwmapiDispatch
f125 endp
f126 proc
  mov r10d, 126
  jmp DwmapiDispatch
f126 endp
f127 proc
  mov r10d, 127
  jmp DwmapiDispatch
f127 endp
f128 proc
  mov r10d, 128
  jmp DwmapiDispatch
f128 endp
f129 proc
  mov r10d, 129
  jmp DwmapiDispatch
f129 endp
f130 proc
  mov r10d, 130
  jmp DwmapiDispatch
f130 endp
f131 proc
  mov r10d, 131
  jmp DwmapiDispatch
f131 endp
f132 proc
  mov r10d, 132
  jmp DwmapiDispatch
f132 endp
f133 proc
  mov r10d, 133
  jmp DwmapiDispatch
f133 endp
f134 proc
  mov r10d, 134
  jmp DwmapiDispatch
f134 endp
f135 proc
  mov r10d, 135
  jmp DwmapiDispatch
f135 endp
f136 proc
  mov r10d, 136
  jmp DwmapiDispatch
f136 endp
f137 proc
  mov r10d, 137
  jmp DwmapiDispatch
f137 endp
f138 proc
  mov r10d, 138
  jmp DwmapiDispatch
f138 endp
f139 proc
  mov r10d, 139
  jmp DwmapiDispatch
f139 endp
f140 proc
  mov r10d, 140
  jmp DwmapiDispatch
f140 endp
f141 proc
  mov r10d, 141
  jmp DwmapiDispatch
f141 endp
f142 proc
  mov r10d, 142
  jmp DwmapiDispatch
f142 endp
end
