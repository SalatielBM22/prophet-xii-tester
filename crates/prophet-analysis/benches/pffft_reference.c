/* SPDX-License-Identifier: GPL-3.0-only
 * Optional independent PFFFT measurement harness. No upstream FFT code copied.
 * Compile with the separately fetched, pinned PFFFT sources documented in
 * docs/analysis/README.md. All alignment/work buffers allocated before timing.
 */
#define _POSIX_C_SOURCE 200809L
#include "pffft.h"
#include <assert.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

static uint64_t ns(void) {
  struct timespec t;
  assert(clock_gettime(CLOCK_MONOTONIC, &t) == 0);
  return (uint64_t)t.tv_sec * 1000000000u + (uint64_t)t.tv_nsec;
}
static int compare(const void *a,const void *b) {
  uint64_t x=*(const uint64_t*)a,y=*(const uint64_t*)b;
  return (x>y)-(x<y);
}
int main(void) {
  puts("method,n,p50_ns,p95_ns,p99_ns,max_ns,retained_buffer_bytes,warmup,iterations");
  for(int n=2048;n<=8192;n*=2) {
    PFFFT_Setup *plan=pffft_new_setup(n,PFFFT_REAL);
    assert(plan);
    float *x=pffft_aligned_malloc(n*sizeof(float));
    float *input=pffft_aligned_malloc(n*sizeof(float));
    float *output=pffft_aligned_malloc(n*sizeof(float));
    float *work=pffft_aligned_malloc(n*sizeof(float));
    float *inverse=pffft_aligned_malloc(n*sizeof(float));
    assert(x&&input&&output&&work&&inverse);
    for(int i=0;i<n;i++)x[i]=(float)sin(6.283185307179586*37.3*i/n);
    pffft_transform_ordered(plan,x,output,work,PFFFT_FORWARD);
    /* Independent direct DFT checks bins, including special DC/Nyquist packing. */
    int bins[]={0,1,36,37,38,n/2};
    for(unsigned j=0;j<sizeof(bins)/sizeof(bins[0]);j++) {
      int k=bins[j];double re=0,im=0;
      for(int i=0;i<n;i++){double p=-6.283185307179586*k*i/n;re+=x[i]*cos(p);im+=x[i]*sin(p);}
      double actual_re=output[k==n/2?1:2*k];
      double actual_im=k==0||k==n/2?0:output[2*k+1];
      assert(fabs(re-actual_re)<0.002 && fabs(im-actual_im)<0.002);
    }
    pffft_transform_ordered(plan,output,inverse,work,PFFFT_BACKWARD);
    for(int i=0;i<n;i++)assert(fabs(inverse[i]/n-x[i])<0.000005);
    for(int i=0;i<100;i++){memcpy(input,x,n*sizeof(float));pffft_transform_ordered(plan,input,output,work,PFFFT_FORWARD);}
    uint64_t timings[2000];
    for(int i=0;i<2000;i++){uint64_t start=ns();memcpy(input,x,n*sizeof(float));pffft_transform_ordered(plan,input,output,work,PFFFT_FORWARD);timings[i]=ns()-start;}
    qsort(timings,2000,sizeof(uint64_t),compare);
    printf("pffft_copy_ordered_r2c,%d,%llu,%llu,%llu,%llu,%d,100,2000\n",n,
      (unsigned long long)timings[1000],(unsigned long long)timings[1900],
      (unsigned long long)timings[1980],(unsigned long long)timings[1999],n*3*4);
    pffft_destroy_setup(plan);pffft_aligned_free(x);pffft_aligned_free(input);
    pffft_aligned_free(output);pffft_aligned_free(work);pffft_aligned_free(inverse);
  }
  return 0;
}
