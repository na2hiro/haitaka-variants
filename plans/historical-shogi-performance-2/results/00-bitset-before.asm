
target/performance2-before-bins/taikyoku:     file format elf64-x86-64


Disassembly of section .text:

000000000004b3b0 <_RNvMs5_NtCs2eU4lUUgvd_7haitaka8extendedINtB5_8PositionNtNtB5_5large9LargeRuleE8set_cellB7_>:
   4b3b0:	55                   	push   %rbp
   4b3b1:	41 57                	push   %r15
   4b3b3:	41 56                	push   %r14
   4b3b5:	41 55                	push   %r13
   4b3b7:	41 54                	push   %r12
   4b3b9:	53                   	push   %rbx
   4b3ba:	48 81 ec 58 01 00 00 	sub    $0x158,%rsp
   4b3c1:	48 89 f3             	mov    %rsi,%rbx
   4b3c4:	48 8b 77 10          	mov    0x10(%rdi),%rsi
   4b3c8:	48 39 f3             	cmp    %rsi,%rbx
   4b3cb:	0f 83 85 01 00 00    	jae    4b556 <_RNvMs5_NtCs2eU4lUUgvd_7haitaka8extendedINtB5_8PositionNtNtB5_5large9LargeRuleE8set_cellB7_+0x1a6>
   4b3d1:	49 89 d7             	mov    %rdx,%r15
   4b3d4:	49 89 fe             	mov    %rdi,%r14
   4b3d7:	48 8b 47 08          	mov    0x8(%rdi),%rax
   4b3db:	48 8d 0c 5b          	lea    (%rbx,%rbx,2),%rcx
   4b3df:	48 8d 2c 48          	lea    (%rax,%rcx,2),%rbp
   4b3e3:	80 3c 48 00          	cmpb   $0x0,(%rax,%rcx,2)
   4b3e7:	0f 84 84 00 00 00    	je     4b471 <_RNvMs5_NtCs2eU4lUUgvd_7haitaka8extendedINtB5_8PositionNtNtB5_5large9LargeRuleE8set_cellB7_+0xc1>
   4b3ed:	0f b6 7d 04          	movzbl 0x4(%rbp),%edi
   4b3f1:	48 83 ff 01          	cmp    $0x1,%rdi
   4b3f5:	0f 87 6a 01 00 00    	ja     4b565 <_RNvMs5_NtCs2eU4lUUgvd_7haitaka8extendedINtB5_8PositionNtNtB5_5large9LargeRuleE8set_cellB7_+0x1b5>
   4b3fb:	48 69 c7 a8 00 00 00 	imul   $0xa8,%rdi,%rax
   4b402:	4d 8d 24 06          	lea    (%r14,%rax,1),%r12
   4b406:	49 83 c4 38          	add    $0x38,%r12
   4b40a:	48 8d 7c 24 08       	lea    0x8(%rsp),%rdi
   4b40f:	ba a8 00 00 00       	mov    $0xa8,%edx
   4b414:	4c 89 e6             	mov    %r12,%rsi
   4b417:	ff 15 2b 45 06 00    	call   *0x6452b(%rip)        # af948 <memcpy@GLIBC_2.14>
   4b41d:	48 81 fb 10 05 00 00 	cmp    $0x510,%rbx
   4b424:	0f 83 fc 00 00 00    	jae    4b526 <_RNvMs5_NtCs2eU4lUUgvd_7haitaka8extendedINtB5_8PositionNtNtB5_5large9LargeRuleE8set_cellB7_+0x176>
   4b42a:	48 c7 c0 fe ff ff ff 	mov    $0xfffffffffffffffe,%rax
   4b431:	89 d9                	mov    %ebx,%ecx
   4b433:	48 d3 c0             	rol    %cl,%rax
   4b436:	48 89 d9             	mov    %rbx,%rcx
   4b439:	48 c1 e9 06          	shr    $0x6,%rcx
   4b43d:	48 21 44 cc 08       	and    %rax,0x8(%rsp,%rcx,8)
   4b442:	48 8d bc 24 b0 00 00 	lea    0xb0(%rsp),%rdi
   4b449:	00 
   4b44a:	48 8d 74 24 08       	lea    0x8(%rsp),%rsi
   4b44f:	4c 8b 2d f2 44 06 00 	mov    0x644f2(%rip),%r13        # af948 <memcpy@GLIBC_2.14>
   4b456:	ba a8 00 00 00       	mov    $0xa8,%edx
   4b45b:	41 ff d5             	call   *%r13
   4b45e:	ba a8 00 00 00       	mov    $0xa8,%edx
   4b463:	4c 89 e7             	mov    %r12,%rdi
   4b466:	48 8d b4 24 b0 00 00 	lea    0xb0(%rsp),%rsi
   4b46d:	00 
   4b46e:	41 ff d5             	call   *%r13
   4b471:	44 89 f9             	mov    %r15d,%ecx
   4b474:	c1 e9 10             	shr    $0x10,%ecx
   4b477:	4c 89 f8             	mov    %r15,%rax
   4b47a:	48 c1 e8 20          	shr    $0x20,%rax
   4b47e:	4c 89 fa             	mov    %r15,%rdx
   4b481:	48 c1 ea 28          	shr    $0x28,%rdx
   4b485:	66 44 89 7d 00       	mov    %r15w,0x0(%rbp)
   4b48a:	66 89 4d 02          	mov    %cx,0x2(%rbp)
   4b48e:	88 45 04             	mov    %al,0x4(%rbp)
   4b491:	88 55 05             	mov    %dl,0x5(%rbp)
   4b494:	41 f6 c7 01          	test   $0x1,%r15b
   4b498:	74 7a                	je     4b514 <_RNvMs5_NtCs2eU4lUUgvd_7haitaka8extendedINtB5_8PositionNtNtB5_5large9LargeRuleE8set_cellB7_+0x164>
   4b49a:	0f b7 c0             	movzwl %ax,%eax
   4b49d:	0f b6 f8             	movzbl %al,%edi
   4b4a0:	48 83 ff 02          	cmp    $0x2,%rdi
   4b4a4:	0f 83 cc 00 00 00    	jae    4b576 <_RNvMs5_NtCs2eU4lUUgvd_7haitaka8extendedINtB5_8PositionNtNtB5_5large9LargeRuleE8set_cellB7_+0x1c6>
   4b4aa:	48 69 c7 a8 00 00 00 	imul   $0xa8,%rdi,%rax
   4b4b1:	49 01 c6             	add    %rax,%r14
   4b4b4:	49 83 c6 38          	add    $0x38,%r14
   4b4b8:	48 8d 7c 24 08       	lea    0x8(%rsp),%rdi
   4b4bd:	ba a8 00 00 00       	mov    $0xa8,%edx
   4b4c2:	4c 89 f6             	mov    %r14,%rsi
   4b4c5:	ff 15 7d 44 06 00    	call   *0x6447d(%rip)        # af948 <memcpy@GLIBC_2.14>
   4b4cb:	48 81 fb 10 05 00 00 	cmp    $0x510,%rbx
   4b4d2:	73 6a                	jae    4b53e <_RNvMs5_NtCs2eU4lUUgvd_7haitaka8extendedINtB5_8PositionNtNtB5_5large9LargeRuleE8set_cellB7_+0x18e>
   4b4d4:	b8 01 00 00 00       	mov    $0x1,%eax
   4b4d9:	89 d9                	mov    %ebx,%ecx
   4b4db:	48 d3 e0             	shl    %cl,%rax
   4b4de:	48 c1 eb 06          	shr    $0x6,%rbx
   4b4e2:	48 09 44 dc 08       	or     %rax,0x8(%rsp,%rbx,8)
   4b4e7:	48 8d 9c 24 b0 00 00 	lea    0xb0(%rsp),%rbx
   4b4ee:	00 
   4b4ef:	48 8d 74 24 08       	lea    0x8(%rsp),%rsi
   4b4f4:	4c 8b 3d 4d 44 06 00 	mov    0x6444d(%rip),%r15        # af948 <memcpy@GLIBC_2.14>
   4b4fb:	ba a8 00 00 00       	mov    $0xa8,%edx
   4b500:	48 89 df             	mov    %rbx,%rdi
   4b503:	41 ff d7             	call   *%r15
   4b506:	ba a8 00 00 00       	mov    $0xa8,%edx
   4b50b:	4c 89 f7             	mov    %r14,%rdi
   4b50e:	48 89 de             	mov    %rbx,%rsi
   4b511:	41 ff d7             	call   *%r15
   4b514:	48 81 c4 58 01 00 00 	add    $0x158,%rsp
   4b51b:	5b                   	pop    %rbx
   4b51c:	41 5c                	pop    %r12
   4b51e:	41 5d                	pop    %r13
   4b520:	41 5e                	pop    %r14
   4b522:	41 5f                	pop    %r15
   4b524:	5d                   	pop    %rbp
   4b525:	c3                   	ret
   4b526:	48 8d 3d a5 66 fc ff 	lea    -0x3995b(%rip),%rdi        # 11bd2 <_RNvNvXs3_NtNtCshPMudpwVzeb_10serde_json5value5indexNtB9_5ValueINtNtNtCsgxBkk5gSRhY_4core3ops5index5IndexpE5index4NULL+0x46a>
   4b52d:	48 8d 15 04 95 05 00 	lea    0x59504(%rip),%rdx        # a4a38 <__frame_dummy_init_array_entry+0x91e8>
   4b534:	be 3b 00 00 00       	mov    $0x3b,%esi
   4b539:	e8 22 2e ff ff       	call   3e360 <_RNvNtCsgxBkk5gSRhY_4core9panicking9panic_fmt>
   4b53e:	48 8d 3d 8d 66 fc ff 	lea    -0x39973(%rip),%rdi        # 11bd2 <_RNvNvXs3_NtNtCshPMudpwVzeb_10serde_json5value5indexNtB9_5ValueINtNtNtCsgxBkk5gSRhY_4core3ops5index5IndexpE5index4NULL+0x46a>
   4b545:	48 8d 15 d4 94 05 00 	lea    0x594d4(%rip),%rdx        # a4a20 <__frame_dummy_init_array_entry+0x91d0>
   4b54c:	be 3b 00 00 00       	mov    $0x3b,%esi
   4b551:	e8 0a 2e ff ff       	call   3e360 <_RNvNtCsgxBkk5gSRhY_4core9panicking9panic_fmt>
   4b556:	48 8d 15 d3 0b 06 00 	lea    0x60bd3(%rip),%rdx        # ac130 <__frame_dummy_init_array_entry+0x108e0>
   4b55d:	48 89 df             	mov    %rbx,%rdi
   4b560:	e8 56 3b ff ff       	call   3f0bb <_RNvNtCsgxBkk5gSRhY_4core9panicking18panic_bounds_check>
   4b565:	48 8d 15 dc 0b 06 00 	lea    0x60bdc(%rip),%rdx        # ac148 <__frame_dummy_init_array_entry+0x108f8>
   4b56c:	be 02 00 00 00       	mov    $0x2,%esi
   4b571:	e8 45 3b ff ff       	call   3f0bb <_RNvNtCsgxBkk5gSRhY_4core9panicking18panic_bounds_check>
   4b576:	48 8d 15 e3 0b 06 00 	lea    0x60be3(%rip),%rdx        # ac160 <__frame_dummy_init_array_entry+0x10910>
   4b57d:	be 02 00 00 00       	mov    $0x2,%esi
   4b582:	e8 34 3b ff ff       	call   3f0bb <_RNvNtCsgxBkk5gSRhY_4core9panicking18panic_bounds_check>
   4b587:	cc                   	int3
   4b588:	cc                   	int3
   4b589:	cc                   	int3
   4b58a:	cc                   	int3
   4b58b:	cc                   	int3
   4b58c:	cc                   	int3
   4b58d:	cc                   	int3
   4b58e:	cc                   	int3
   4b58f:	cc                   	int3
