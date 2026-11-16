
target/perft-position-benchmark/bins/taikyoku:     file format elf64-x86-64


Disassembly of section .text:

0000000000048fb0 <haitaka::extended::large::hop>:
   48fb0:	55                   	push   %rbp
   48fb1:	41 57                	push   %r15
   48fb3:	41 56                	push   %r14
   48fb5:	41 55                	push   %r13
   48fb7:	41 54                	push   %r12
   48fb9:	53                   	push   %rbx
   48fba:	48 83 ec 58          	sub    $0x58,%rsp
   48fbe:	49 89 f4             	mov    %rsi,%r12
   48fc1:	48 8b 77 10          	mov    0x10(%rdi),%rsi
   48fc5:	49 39 f4             	cmp    %rsi,%r12
   48fc8:	0f 83 f5 02 00 00    	jae    492c3 <haitaka::extended::large::hop+0x313>
   48fce:	49 89 d6             	mov    %rdx,%r14
   48fd1:	48 89 fd             	mov    %rdi,%rbp
   48fd4:	48 8b 47 08          	mov    0x8(%rdi),%rax
   48fd8:	4b 8d 14 64          	lea    (%r12,%r12,2),%rdx
   48fdc:	8b 1c 50             	mov    (%rax,%rdx,2),%ebx
   48fdf:	f6 c3 01             	test   $0x1,%bl
   48fe2:	0f 84 b7 02 00 00    	je     4929f <haitaka::extended::large::hop+0x2ef>
   48fe8:	49 39 f6             	cmp    %rsi,%r14
   48feb:	0f 83 e1 02 00 00    	jae    492d2 <haitaka::extended::large::hop+0x322>
   48ff1:	4d 89 c7             	mov    %r8,%r15
   48ff4:	44 0f b6 44 50 04    	movzbl 0x4(%rax,%rdx,2),%r8d
   48ffa:	0f b6 74 50 05       	movzbl 0x5(%rax,%rdx,2),%esi
   48fff:	4b 8d 14 76          	lea    (%r14,%r14,2),%rdx
   49003:	f6 04 50 01          	testb  $0x1,(%rax,%rdx,2)
   49007:	40 88 74 24 0b       	mov    %sil,0xb(%rsp)
   4900c:	44 88 44 24 0a       	mov    %r8b,0xa(%rsp)
   49011:	74 6c                	je     4907f <haitaka::extended::large::hop+0xcf>
   49013:	48 8d 04 50          	lea    (%rax,%rdx,2),%rax
   49017:	0f b7 78 02          	movzwl 0x2(%rax),%edi
   4901b:	48 81 ff b2 01 00 00 	cmp    $0x1b2,%rdi
   49022:	0f 87 b9 02 00 00    	ja     492e1 <haitaka::extended::large::hop+0x331>
   49028:	48 8d 14 bf          	lea    (%rdi,%rdi,4),%rdx
   4902c:	c1 e2 04             	shl    $0x4,%edx
   4902f:	48 8d 05 32 39 05 00 	lea    0x53932(%rip),%rax        # 9c968 <__frame_dummy_init_array_entry+0x998>
   49036:	48 8b 14 02          	mov    (%rdx,%rax,1),%rdx
   4903a:	66 81 3a 62 6c       	cmpw   $0x6c62,(%rdx)
   4903f:	74 07                	je     49048 <haitaka::extended::large::hop+0x98>
   49041:	66 81 3a 64 68       	cmpw   $0x6864,(%rdx)
   49046:	75 37                	jne    4907f <haitaka::extended::large::hop+0xcf>
   49048:	89 df                	mov    %ebx,%edi
   4904a:	c1 ef 10             	shr    $0x10,%edi
   4904d:	81 fb ff ff b2 01    	cmp    $0x1b2ffff,%ebx
   49053:	0f 87 88 02 00 00    	ja     492e1 <haitaka::extended::large::hop+0x331>
   49059:	48 8d 14 bf          	lea    (%rdi,%rdi,4),%rdx
   4905d:	c1 e2 04             	shl    $0x4,%edx
   49060:	48 8b 04 02          	mov    (%rdx,%rax,1),%rax
   49064:	66 81 38 62 6c       	cmpw   $0x6c62,(%rax)
   49069:	74 14                	je     4907f <haitaka::extended::large::hop+0xcf>
   4906b:	66 81 38 64 68       	cmpw   $0x6864,(%rax)
   49070:	0f 95 c0             	setne  %al
   49073:	88 45 30             	mov    %al,0x30(%rbp)
   49076:	84 c9                	test   %cl,%cl
   49078:	75 12                	jne    4908c <haitaka::extended::large::hop+0xdc>
   4907a:	e9 da 01 00 00       	jmp    49259 <haitaka::extended::large::hop+0x2a9>
   4907f:	31 c0                	xor    %eax,%eax
   49081:	88 45 30             	mov    %al,0x30(%rbp)
   49084:	84 c9                	test   %cl,%cl
   49086:	0f 84 cd 01 00 00    	je     49259 <haitaka::extended::large::hop+0x2a9>
   4908c:	48 b9 8f e3 38 8e e3 	movabs $0xe38e38e38e38e38f,%rcx
   49093:	38 8e e3 
   49096:	4c 89 e0             	mov    %r12,%rax
   49099:	48 f7 e1             	mul    %rcx
   4909c:	49 81 fc 10 05 00 00 	cmp    $0x510,%r12
   490a3:	0f 83 02 02 00 00    	jae    492ab <haitaka::extended::large::hop+0x2fb>
   490a9:	49 81 fe 10 05 00 00 	cmp    $0x510,%r14
   490b0:	0f 83 01 02 00 00    	jae    492b7 <haitaka::extended::large::hop+0x307>
   490b6:	89 5c 24 0c          	mov    %ebx,0xc(%rsp)
   490ba:	48 89 6c 24 38       	mov    %rbp,0x38(%rsp)
   490bf:	4c 89 7c 24 40       	mov    %r15,0x40(%rsp)
   490c4:	48 c1 ea 05          	shr    $0x5,%rdx
   490c8:	48 8d 04 95 00 00 00 	lea    0x0(,%rdx,4),%rax
   490cf:	00 
   490d0:	48 8d 04 c0          	lea    (%rax,%rax,8),%rax
   490d4:	4c 89 e5             	mov    %r12,%rbp
   490d7:	48 29 c5             	sub    %rax,%rbp
   490da:	41 69 de 8f e3 00 00 	imul   $0xe38f,%r14d,%ebx
   490e1:	c1 eb 15             	shr    $0x15,%ebx
   490e4:	8d 04 9d 00 00 00 00 	lea    0x0(,%rbx,4),%eax
   490eb:	8d 04 c0             	lea    (%rax,%rax,8),%eax
   490ee:	4c 89 74 24 28       	mov    %r14,0x28(%rsp)
   490f3:	44 89 f1             	mov    %r14d,%ecx
   490f6:	29 c1                	sub    %eax,%ecx
   490f8:	0f b7 f1             	movzwl %cx,%esi
   490fb:	48 89 54 24 50       	mov    %rdx,0x50(%rsp)
   49100:	48 39 d3             	cmp    %rdx,%rbx
   49103:	0f 9c c0             	setl   %al
   49106:	0f 9f c1             	setg   %cl
   49109:	28 c1                	sub    %al,%cl
   4910b:	48 0f be c1          	movsbq %cl,%rax
   4910f:	48 89 44 24 48       	mov    %rax,0x48(%rsp)
   49114:	49 89 f7             	mov    %rsi,%r15
   49117:	48 39 ee             	cmp    %rbp,%rsi
   4911a:	0f 9c c0             	setl   %al
   4911d:	0f 9f c1             	setg   %cl
   49120:	28 c1                	sub    %al,%cl
   49122:	4c 0f be e9          	movsbq %cl,%r13
   49126:	48 c7 44 24 10 00 00 	movq   $0x0,0x10(%rsp)
   4912d:	00 00 
   4912f:	48 c7 44 24 18 08 00 	movq   $0x8,0x18(%rsp)
   49136:	00 00 
   49138:	48 c7 44 24 20 00 00 	movq   $0x0,0x20(%rsp)
   4913f:	00 00 
   49141:	b8 08 00 00 00       	mov    $0x8,%eax
   49146:	4d 89 e6             	mov    %r12,%r14
   49149:	45 31 e4             	xor    %r12d,%r12d
   4914c:	4c 89 74 24 30       	mov    %r14,0x30(%rsp)
   49151:	eb 2b                	jmp    4917e <haitaka::extended::large::hop+0x1ce>
   49153:	66 66 66 66 2e 0f 1f 	data16 data16 data16 cs nopw 0x0(%rax,%rax,1)
   4915a:	84 00 00 00 00 00 
   49160:	48 8b 44 24 18       	mov    0x18(%rsp),%rax
   49165:	4e 89 34 e0          	mov    %r14,(%rax,%r12,8)
   49169:	49 ff c4             	inc    %r12
   4916c:	4c 89 64 24 20       	mov    %r12,0x20(%rsp)
   49171:	49 81 fe 10 05 00 00 	cmp    $0x510,%r14
   49178:	0f 83 82 00 00 00    	jae    49200 <haitaka::extended::large::hop+0x250>
   4917e:	48 39 5c 24 50       	cmp    %rbx,0x50(%rsp)
   49183:	40 0f 97 c6          	seta   %sil
   49187:	41 69 d6 8f e3 00 00 	imul   $0xe38f,%r14d,%edx
   4918e:	c1 ea 15             	shr    $0x15,%edx
   49191:	48 89 d1             	mov    %rdx,%rcx
   49194:	48 03 4c 24 48       	add    0x48(%rsp),%rcx
   49199:	40 0f 92 c7          	setb   %dil
   4919d:	40 30 f7             	xor    %sil,%dil
   491a0:	75 5e                	jne    49200 <haitaka::extended::large::hop+0x250>
   491a2:	c1 e2 02             	shl    $0x2,%edx
   491a5:	8d 14 d2             	lea    (%rdx,%rdx,8),%edx
   491a8:	41 29 d6             	sub    %edx,%r14d
   491ab:	4c 39 fd             	cmp    %r15,%rbp
   491ae:	40 0f 97 c6          	seta   %sil
   491b2:	41 0f b7 d6          	movzwl %r14w,%edx
   491b6:	4c 01 ea             	add    %r13,%rdx
   491b9:	40 0f 92 c7          	setb   %dil
   491bd:	40 30 f7             	xor    %sil,%dil
   491c0:	75 3e                	jne    49200 <haitaka::extended::large::hop+0x250>
   491c2:	48 83 f9 24          	cmp    $0x24,%rcx
   491c6:	40 0f 93 c6          	setae  %sil
   491ca:	48 83 fa 24          	cmp    $0x24,%rdx
   491ce:	40 0f 93 c7          	setae  %dil
   491d2:	40 08 f7             	or     %sil,%dil
   491d5:	75 29                	jne    49200 <haitaka::extended::large::hop+0x250>
   491d7:	48 8d 0c c9          	lea    (%rcx,%rcx,8),%rcx
   491db:	4c 8d 34 8a          	lea    (%rdx,%rcx,4),%r14
   491df:	4c 3b 74 24 28       	cmp    0x28(%rsp),%r14
   491e4:	74 1a                	je     49200 <haitaka::extended::large::hop+0x250>
   491e6:	4c 3b 64 24 10       	cmp    0x10(%rsp),%r12
   491eb:	0f 85 74 ff ff ff    	jne    49165 <haitaka::extended::large::hop+0x1b5>
   491f1:	48 8d 7c 24 10       	lea    0x10(%rsp),%rdi
   491f6:	e8 55 11 00 00       	call   4a350 <<alloc::raw_vec::RawVec<usize>>::grow_one>
   491fb:	e9 60 ff ff ff       	jmp    49160 <haitaka::extended::large::hop+0x1b0>
   49200:	4c 8b 74 24 10       	mov    0x10(%rsp),%r14
   49205:	4c 8b 6c 24 18       	mov    0x18(%rsp),%r13
   4920a:	49 c1 e4 03          	shl    $0x3,%r12
   4920e:	31 db                	xor    %ebx,%ebx
   49210:	4c 8b 7c 24 40       	mov    0x40(%rsp),%r15
   49215:	48 8b 6c 24 38       	mov    0x38(%rsp),%rbp
   4921a:	66 0f 1f 44 00 00    	nopw   0x0(%rax,%rax,1)
   49220:	49 39 dc             	cmp    %rbx,%r12
   49223:	74 18                	je     4923d <haitaka::extended::large::hop+0x28d>
   49225:	49 8b 54 1d 00       	mov    0x0(%r13,%rbx,1),%rdx
   4922a:	48 83 c3 08          	add    $0x8,%rbx
   4922e:	4c 89 ff             	mov    %r15,%rdi
   49231:	48 89 ee             	mov    %rbp,%rsi
   49234:	31 c9                	xor    %ecx,%ecx
   49236:	e8 e5 00 00 00       	call   49320 <<haitaka::extended::large::Delta>::set>
   4923b:	eb e3                	jmp    49220 <haitaka::extended::large::hop+0x270>
   4923d:	4d 85 f6             	test   %r14,%r14
   49240:	4c 8b 74 24 28       	mov    0x28(%rsp),%r14
   49245:	4c 8b 64 24 30       	mov    0x30(%rsp),%r12
   4924a:	8b 5c 24 0c          	mov    0xc(%rsp),%ebx
   4924e:	74 09                	je     49259 <haitaka::extended::large::hop+0x2a9>
   49250:	4c 89 ef             	mov    %r13,%rdi
   49253:	ff 15 a7 6e 06 00    	call   *0x66ea7(%rip)        # b0100 <free@GLIBC_2.2.5>
   49259:	89 db                	mov    %ebx,%ebx
   4925b:	4c 89 ff             	mov    %r15,%rdi
   4925e:	48 89 ee             	mov    %rbp,%rsi
   49261:	4c 89 e2             	mov    %r12,%rdx
   49264:	31 c9                	xor    %ecx,%ecx
   49266:	e8 b5 00 00 00       	call   49320 <<haitaka::extended::large::Delta>::set>
   4926b:	0f b6 44 24 0a       	movzbl 0xa(%rsp),%eax
   49270:	48 c1 e0 20          	shl    $0x20,%rax
   49274:	48 09 d8             	or     %rbx,%rax
   49277:	0f b6 4c 24 0b       	movzbl 0xb(%rsp),%ecx
   4927c:	48 c1 e1 28          	shl    $0x28,%rcx
   49280:	48 09 c1             	or     %rax,%rcx
   49283:	4c 89 ff             	mov    %r15,%rdi
   49286:	48 89 ee             	mov    %rbp,%rsi
   49289:	4c 89 f2             	mov    %r14,%rdx
   4928c:	48 83 c4 58          	add    $0x58,%rsp
   49290:	5b                   	pop    %rbx
   49291:	41 5c                	pop    %r12
   49293:	41 5d                	pop    %r13
   49295:	41 5e                	pop    %r14
   49297:	41 5f                	pop    %r15
   49299:	5d                   	pop    %rbp
   4929a:	e9 81 00 00 00       	jmp    49320 <<haitaka::extended::large::Delta>::set>
   4929f:	48 8d 3d 1a 38 06 00 	lea    0x6381a(%rip),%rdi        # acac0 <__frame_dummy_init_array_entry+0x10af0>
   492a6:	e8 d5 82 ff ff       	call   41580 <core::option::unwrap_failed>
   492ab:	48 8d 3d ae 37 06 00 	lea    0x637ae(%rip),%rdi        # aca60 <__frame_dummy_init_array_entry+0x10a90>
   492b2:	e8 c9 82 ff ff       	call   41580 <core::option::unwrap_failed>
   492b7:	48 8d 3d ba 37 06 00 	lea    0x637ba(%rip),%rdi        # aca78 <__frame_dummy_init_array_entry+0x10aa8>
   492be:	e8 bd 82 ff ff       	call   41580 <core::option::unwrap_failed>
   492c3:	48 8d 15 de 37 06 00 	lea    0x637de(%rip),%rdx        # acaa8 <__frame_dummy_init_array_entry+0x10ad8>
   492ca:	4c 89 e7             	mov    %r12,%rdi
   492cd:	e8 39 62 ff ff       	call   3f50b <core::panicking::panic_bounds_check>
   492d2:	48 8d 15 ff 37 06 00 	lea    0x637ff(%rip),%rdx        # acad8 <__frame_dummy_init_array_entry+0x10b08>
   492d9:	4c 89 f7             	mov    %r14,%rdi
   492dc:	e8 2a 62 ff ff       	call   3f50b <core::panicking::panic_bounds_check>
   492e1:	48 8d 15 50 36 05 00 	lea    0x53650(%rip),%rdx        # 9c938 <__frame_dummy_init_array_entry+0x968>
   492e8:	be b3 01 00 00       	mov    $0x1b3,%esi
   492ed:	e8 19 62 ff ff       	call   3f50b <core::panicking::panic_bounds_check>
   492f2:	48 89 c3             	mov    %rax,%rbx
   492f5:	48 83 7c 24 10 00    	cmpq   $0x0,0x10(%rsp)
   492fb:	74 18                	je     49315 <haitaka::extended::large::hop+0x365>
   492fd:	4c 8b 6c 24 18       	mov    0x18(%rsp),%r13
   49302:	eb 08                	jmp    4930c <haitaka::extended::large::hop+0x35c>
   49304:	48 89 c3             	mov    %rax,%rbx
   49307:	4d 85 f6             	test   %r14,%r14
   4930a:	74 09                	je     49315 <haitaka::extended::large::hop+0x365>
   4930c:	4c 89 ef             	mov    %r13,%rdi
   4930f:	ff 15 eb 6d 06 00    	call   *0x66deb(%rip)        # b0100 <free@GLIBC_2.2.5>
   49315:	48 89 df             	mov    %rbx,%rdi
   49318:	e8 93 1c 05 00       	call   9afb0 <_Unwind_Resume@plt>
