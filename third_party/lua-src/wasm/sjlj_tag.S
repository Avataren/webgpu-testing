# The exception tag that clang's wasm setjmp/longjmp lowering throws and catches.
# Its payload is a pointer to the jmp_buf's {env, val} pair.
.globl __c_longjmp
.tagtype __c_longjmp i32
__c_longjmp:
