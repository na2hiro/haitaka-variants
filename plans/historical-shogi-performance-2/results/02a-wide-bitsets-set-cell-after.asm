
target/perft-position-benchmark/bins/taikyoku:     file format elf64-x86-64


Disassembly of section .text:

000000000004a1e0 <<haitaka::extended::Position<haitaka::extended::large::LargeRule>>::set_cell>:
   4a1e0:	41 56                	push   %r14
   4a1e2:	53                   	push   %rbx
   4a1e3:	50                   	push   %rax
   4a1e4:	48 8b 47 10          	mov    0x10(%rdi),%rax
   4a1e8:	48 39 c6             	cmp    %rax,%rsi
   4a1eb:	0f 83 83 01 00 00    	jae    4a374 <<haitaka::extended::Position<haitaka::extended::large::LargeRule>>::set_cell+0x194>
   4a1f1:	49 b8 ff ff ff ff ff 	movabs $0xffffffffffff,%r8
   4a1f8:	ff 00 00 
   4a1fb:	49 21 d0             	and    %rdx,%r8
   4a1fe:	4d 89 c1             	mov    %r8,%r9
   4a201:	49 c1 e9 10          	shr    $0x10,%r9
   4a205:	49 c1 e8 20          	shr    $0x20,%r8
   4a209:	49 89 d2             	mov    %rdx,%r10
   4a20c:	49 c1 ea 28          	shr    $0x28,%r10
   4a210:	48 8b 47 08          	mov    0x8(%rdi),%rax
   4a214:	48 8d 0c 76          	lea    (%rsi,%rsi,2),%rcx
   4a218:	4c 8d 1c 48          	lea    (%rax,%rcx,2),%r11
   4a21c:	80 3c 48 00          	cmpb   $0x0,(%rax,%rcx,2)
   4a220:	74 54                	je     4a276 <<haitaka::extended::Position<haitaka::extended::large::LargeRule>>::set_cell+0x96>
   4a222:	41 0f b6 4b 04       	movzbl 0x4(%r11),%ecx
   4a227:	41 0f b7 43 02       	movzwl 0x2(%r11),%eax
   4a22c:	f6 c2 01             	test   $0x1,%dl
   4a22f:	74 0e                	je     4a23f <<haitaka::extended::Position<haitaka::extended::large::LargeRule>>::set_cell+0x5f>
   4a231:	44 38 c1             	cmp    %r8b,%cl
   4a234:	75 09                	jne    4a23f <<haitaka::extended::Position<haitaka::extended::large::LargeRule>>::set_cell+0x5f>
   4a236:	44 89 c1             	mov    %r8d,%ecx
   4a239:	66 44 39 c8          	cmp    %r9w,%ax
   4a23d:	74 3c                	je     4a27b <<haitaka::extended::Position<haitaka::extended::large::LargeRule>>::set_cell+0x9b>
   4a23f:	3d b2 01 00 00       	cmp    $0x1b2,%eax
   4a244:	0f 87 3c 01 00 00    	ja     4a386 <<haitaka::extended::Position<haitaka::extended::large::LargeRule>>::set_cell+0x1a6>
   4a24a:	48 8d 1c 80          	lea    (%rax,%rax,4),%rbx
   4a24e:	c1 e3 04             	shl    $0x4,%ebx
   4a251:	4c 8d 35 b0 27 05 00 	lea    0x527b0(%rip),%r14        # 9ca08 <__frame_dummy_init_array_entry+0x998>
   4a258:	0f b6 c1             	movzbl %cl,%eax
   4a25b:	42 80 7c 33 48 00    	cmpb   $0x0,0x48(%rbx,%r14,1)
   4a261:	74 20                	je     4a283 <<haitaka::extended::Position<haitaka::extended::large::LargeRule>>::set_cell+0xa3>
   4a263:	80 f9 02             	cmp    $0x2,%cl
   4a266:	0f 83 2e 01 00 00    	jae    4a39a <<haitaka::extended::Position<haitaka::extended::large::LargeRule>>::set_cell+0x1ba>
   4a26c:	66 ff 8c 47 88 01 00 	decw   0x188(%rdi,%rax,2)
   4a273:	00 
   4a274:	eb 16                	jmp    4a28c <<haitaka::extended::Position<haitaka::extended::large::LargeRule>>::set_cell+0xac>
   4a276:	66 85 d2             	test   %dx,%dx
   4a279:	75 40                	jne    4a2bb <<haitaka::extended::Position<haitaka::extended::large::LargeRule>>::set_cell+0xdb>
   4a27b:	48 83 c4 08          	add    $0x8,%rsp
   4a27f:	5b                   	pop    %rbx
   4a280:	41 5e                	pop    %r14
   4a282:	c3                   	ret
   4a283:	80 f9 02             	cmp    $0x2,%cl
   4a286:	0f 83 22 01 00 00    	jae    4a3ae <<haitaka::extended::Position<haitaka::extended::large::LargeRule>>::set_cell+0x1ce>
   4a28c:	48 81 fe 10 05 00 00 	cmp    $0x510,%rsi
   4a293:	0f 83 ab 00 00 00    	jae    4a344 <<haitaka::extended::Position<haitaka::extended::large::LargeRule>>::set_cell+0x164>
   4a299:	48 c7 c3 fe ff ff ff 	mov    $0xfffffffffffffffe,%rbx
   4a2a0:	89 f1                	mov    %esi,%ecx
   4a2a2:	48 d3 c3             	rol    %cl,%rbx
   4a2a5:	48 69 c0 a8 00 00 00 	imul   $0xa8,%rax,%rax
   4a2ac:	48 01 f8             	add    %rdi,%rax
   4a2af:	48 89 f1             	mov    %rsi,%rcx
   4a2b2:	48 c1 e9 06          	shr    $0x6,%rcx
   4a2b6:	48 21 5c c8 38       	and    %rbx,0x38(%rax,%rcx,8)
   4a2bb:	66 41 89 13          	mov    %dx,(%r11)
   4a2bf:	66 45 89 4b 02       	mov    %r9w,0x2(%r11)
   4a2c4:	45 88 43 04          	mov    %r8b,0x4(%r11)
   4a2c8:	45 88 53 05          	mov    %r10b,0x5(%r11)
   4a2cc:	f6 c2 01             	test   $0x1,%dl
   4a2cf:	74 aa                	je     4a27b <<haitaka::extended::Position<haitaka::extended::large::LargeRule>>::set_cell+0x9b>
   4a2d1:	41 0f b7 c1          	movzwl %r9w,%eax
   4a2d5:	48 3d b3 01 00 00    	cmp    $0x1b3,%rax
   4a2db:	0f 83 a5 00 00 00    	jae    4a386 <<haitaka::extended::Position<haitaka::extended::large::LargeRule>>::set_cell+0x1a6>
   4a2e1:	48 8d 0c 80          	lea    (%rax,%rax,4),%rcx
   4a2e5:	c1 e1 04             	shl    $0x4,%ecx
   4a2e8:	48 8d 15 19 27 05 00 	lea    0x52719(%rip),%rdx        # 9ca08 <__frame_dummy_init_array_entry+0x998>
   4a2ef:	41 0f b6 c0          	movzbl %r8b,%eax
   4a2f3:	80 7c 11 48 00       	cmpb   $0x0,0x48(%rcx,%rdx,1)
   4a2f8:	74 13                	je     4a30d <<haitaka::extended::Position<haitaka::extended::large::LargeRule>>::set_cell+0x12d>
   4a2fa:	83 f8 02             	cmp    $0x2,%eax
   4a2fd:	0f 83 bf 00 00 00    	jae    4a3c2 <<haitaka::extended::Position<haitaka::extended::large::LargeRule>>::set_cell+0x1e2>
   4a303:	66 ff 84 47 88 01 00 	incw   0x188(%rdi,%rax,2)
   4a30a:	00 
   4a30b:	eb 09                	jmp    4a316 <<haitaka::extended::Position<haitaka::extended::large::LargeRule>>::set_cell+0x136>
   4a30d:	83 f8 02             	cmp    $0x2,%eax
   4a310:	0f 83 c0 00 00 00    	jae    4a3d6 <<haitaka::extended::Position<haitaka::extended::large::LargeRule>>::set_cell+0x1f6>
   4a316:	48 81 fe 10 05 00 00 	cmp    $0x510,%rsi
   4a31d:	73 3d                	jae    4a35c <<haitaka::extended::Position<haitaka::extended::large::LargeRule>>::set_cell+0x17c>
   4a31f:	48 69 c0 a8 00 00 00 	imul   $0xa8,%rax,%rax
   4a326:	48 01 c7             	add    %rax,%rdi
   4a329:	b8 01 00 00 00       	mov    $0x1,%eax
   4a32e:	89 f1                	mov    %esi,%ecx
   4a330:	48 d3 e0             	shl    %cl,%rax
   4a333:	48 c1 ee 06          	shr    $0x6,%rsi
   4a337:	48 09 44 f7 38       	or     %rax,0x38(%rdi,%rsi,8)
   4a33c:	48 83 c4 08          	add    $0x8,%rsp
   4a340:	5b                   	pop    %rbx
   4a341:	41 5e                	pop    %r14
   4a343:	c3                   	ret
   4a344:	48 8d 3d 0f 79 fc ff 	lea    -0x386f1(%rip),%rdi        # 11c5a <<serde_json::value::Value as core::ops::index::Index<_>>::index::NULL+0x46a>
   4a34b:	48 8d 15 2e 29 06 00 	lea    0x6292e(%rip),%rdx        # acc80 <__frame_dummy_init_array_entry+0x10c10>
   4a352:	be 3b 00 00 00       	mov    $0x3b,%esi
   4a357:	e8 b4 44 ff ff       	call   3e810 <core::panicking::panic_fmt>
   4a35c:	48 8d 3d f7 78 fc ff 	lea    -0x38709(%rip),%rdi        # 11c5a <<serde_json::value::Value as core::ops::index::Index<_>>::index::NULL+0x46a>
   4a363:	48 8d 15 fe 28 06 00 	lea    0x628fe(%rip),%rdx        # acc68 <__frame_dummy_init_array_entry+0x10bf8>
   4a36a:	be 3b 00 00 00       	mov    $0x3b,%esi
   4a36f:	e8 9c 44 ff ff       	call   3e810 <core::panicking::panic_fmt>
   4a374:	48 8d 15 bd 25 06 00 	lea    0x625bd(%rip),%rdx        # ac938 <__frame_dummy_init_array_entry+0x108c8>
   4a37b:	48 89 f7             	mov    %rsi,%rdi
   4a37e:	48 89 c6             	mov    %rax,%rsi
   4a381:	e8 e5 51 ff ff       	call   3f56b <core::panicking::panic_bounds_check>
   4a386:	48 8d 15 4b 26 05 00 	lea    0x5264b(%rip),%rdx        # 9c9d8 <__frame_dummy_init_array_entry+0x968>
   4a38d:	be b3 01 00 00       	mov    $0x1b3,%esi
   4a392:	48 89 c7             	mov    %rax,%rdi
   4a395:	e8 d1 51 ff ff       	call   3f56b <core::panicking::panic_bounds_check>
   4a39a:	48 8d 15 27 26 06 00 	lea    0x62627(%rip),%rdx        # ac9c8 <__frame_dummy_init_array_entry+0x10958>
   4a3a1:	be 02 00 00 00       	mov    $0x2,%esi
   4a3a6:	48 89 c7             	mov    %rax,%rdi
   4a3a9:	e8 bd 51 ff ff       	call   3f56b <core::panicking::panic_bounds_check>
   4a3ae:	48 8d 15 9b 25 06 00 	lea    0x6259b(%rip),%rdx        # ac950 <__frame_dummy_init_array_entry+0x108e0>
   4a3b5:	be 02 00 00 00       	mov    $0x2,%esi
   4a3ba:	48 89 c7             	mov    %rax,%rdi
   4a3bd:	e8 a9 51 ff ff       	call   3f56b <core::panicking::panic_bounds_check>
   4a3c2:	48 8d 15 17 26 06 00 	lea    0x62617(%rip),%rdx        # ac9e0 <__frame_dummy_init_array_entry+0x10970>
   4a3c9:	be 02 00 00 00       	mov    $0x2,%esi
   4a3ce:	48 89 c7             	mov    %rax,%rdi
   4a3d1:	e8 95 51 ff ff       	call   3f56b <core::panicking::panic_bounds_check>
   4a3d6:	48 8d 15 8b 25 06 00 	lea    0x6258b(%rip),%rdx        # ac968 <__frame_dummy_init_array_entry+0x108f8>
   4a3dd:	be 02 00 00 00       	mov    $0x2,%esi
   4a3e2:	48 89 c7             	mov    %rax,%rdi
   4a3e5:	e8 81 51 ff ff       	call   3f56b <core::panicking::panic_bounds_check>
