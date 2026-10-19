#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdio.h>
static int native_initialized;
__attribute__((constructor)) static void native_initialize(void){native_initialized=42;}
void native_output(void){puts("Native lifecycle passed");}
struct Small { uint8_t a; int32_t b; };
struct Mixed { double a; uint32_t b; };
struct Large { uint8_t a; uint64_t b; float c; };
struct Nested { uint16_t a; struct Small b; uint8_t c[3]; };
struct Pair { uint64_t a,b; };
struct PackedFloats { float data[3]; uint8_t tag; };
_Static_assert(sizeof(struct Small)==8 && offsetof(struct Small,b)==4,"small layout");
_Static_assert(sizeof(struct Mixed)==16 && offsetof(struct Mixed,b)==8,"mixed layout");
_Static_assert(sizeof(struct Large)==24 && offsetof(struct Large,b)==8 && offsetof(struct Large,c)==16,"large layout");
_Static_assert(sizeof(struct Nested)==16 && offsetof(struct Nested,b)==4 && offsetof(struct Nested,c)==12,"nested layout");
_Static_assert(sizeof(struct PackedFloats)==16 && offsetof(struct PackedFloats,tag)==12,"packed float layout");
extern int32_t jr_callback(int32_t);
extern struct Small jr_small(struct Small);
extern struct Mixed jr_mixed(struct Mixed);
extern struct Large jr_large(struct Large);
extern struct Nested jr_nested(struct Nested);
extern struct PackedFloats jr_packed(struct PackedFloats);
extern __int128 jr_wide(__int128,uint64_t,__int128,uint64_t,__int128);
extern uint64_t jr_rollback(uint64_t,uint64_t,uint64_t,uint64_t,uint64_t,struct Pair,uint64_t);
extern double jr_floats(double,double,double,double,double,double,double,double,double,double);
uint64_t native_scalars(int8_t a,uint16_t b,int32_t c,uint64_t d,bool e,float f,double g,uint64_t h,uint64_t i,uint64_t j){return a==-7&&b==65000&&c==-123456&&d==UINT64_C(0xfedcba9876543210)&&e&&f==1.5f&&g==2.25&&h==8&&i==9&&j==10?12345:0;}
double native_floats(double a,double b,double c,double d,double e,double f,double g,double h,double i,double j){return a+b+c+d+e+f+g+h+i+j;}
__int128 native_wide(__int128 a,uint64_t b,__int128 c,uint64_t d,__int128 e){return a+b+c+d+e;}
struct Small native_small(struct Small value){return jr_small(value);}
struct Mixed native_mixed(struct Mixed value){return jr_mixed(value);}
struct Large native_large(struct Large value){return jr_large(value);}
struct Nested native_nested(struct Nested value){return jr_nested(value);}
struct PackedFloats native_packed(struct PackedFloats value){return jr_packed(value);}
uint64_t native_rollback(uint64_t a,uint64_t b,uint64_t c,uint64_t d,uint64_t e,struct Pair p,uint64_t last){return a+b+c+d+e+p.a+p.b+last;}
int32_t native_add(int32_t a,int32_t b){return a+b;}
/* Volatile callback storage forces an actual indirect C -> generated Rust call. */
int32_t native_roundtrip(int32_t value){if(native_initialized!=42)return -10000;int32_t (*volatile callback)(int32_t)=jr_callback;return callback(value);}
#ifdef G6_C_MAIN
int main(void){
    struct Small small=jr_small((struct Small){7,-1234});if(small.a!=8||small.b!=-1236)return 1;
    struct Mixed mixed=jr_mixed((struct Mixed){2.5,9});if(mixed.a!=4.0||mixed.b!=16)return 2;
    struct Large large=jr_large((struct Large){3,123,2.5f});if(large.a!=4||large.b!=125||large.c!=3.0f)return 3;
    struct Nested nested=jr_nested((struct Nested){10,{20,-30},{1,2,3}});if(nested.a!=11||nested.b.a!=22||nested.b.b!=-27||nested.c[0]!=2||nested.c[1]!=4||nested.c[2]!=6)return 4;
    if(jr_wide(((__int128)1<<64)+1,2,-((__int128)1<<64),3,5)!=11)return 5;
    if(jr_rollback(1,2,3,4,5,(struct Pair){6,7},8)!=36)return 6;
    if(jr_floats(1,2,3,4,5,6,7,8,9,10)!=55)return 7;
    struct PackedFloats packed=jr_packed((struct PackedFloats){{1,2,3},4});if(packed.data[0]!=2||packed.data[1]!=4||packed.data[2]!=6||packed.tag!=5)return 8;
    if(native_roundtrip(20)!=29)return 9;
    puts("C entry passed");return 0;
}
#endif
