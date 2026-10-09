// 本机（macOS）给 switchgl 的 GLSL ES 1.00 着色器做语法检查。
//
// 这些着色器只在真机运行时才编译，编不过只会在日志里留一行，所以改完很容易
// 带着错误进仓库。这里用 macOS 传统 GL 上下文按 GLSL 1.20 编一遍——内置函数
// 重载、保留字、语句语法这几类判定和 ES 1.00 一致，真实抓到过两个：
//
//   * `max(int, int)`：ES 1.00 的 max 只有浮点重载；
//   * 拿 `packed` 当变量名：它是 ES 1.00 的保留字。
//
// 用法（在仓库根目录）：
//
//   clang -o /tmp/es2check switch/tools/es2check.c -framework OpenGL
//   /tmp/es2check radiance/radiance/src/rendering/switchgl/shaders/*.vert \
//                 radiance/radiance/src/rendering/switchgl/shaders/*.frag
//
// 阶段按扩展名认（`.frag` 当片元，其余当顶点）。
//
// **不是等价校验**：GLSL 1.20 比 ES 1.00 宽松（例如允许 `mat3(mat4)` 这种降阶
// 构造、不要求 `precision` 限定符），所以通过不等于上机一定能编过；反过来，这里
// 报错基本就是真错。精度限定符和 `#version` 行会被剥掉再编。

#include <OpenGL/OpenGL.h>
#include <OpenGL/gl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static char *slurp(const char *path, long *len) {
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    *len = ftell(f);
    fseek(f, 0, SEEK_SET);
    char *buf = malloc(*len + 1);
    if (fread(buf, 1, *len, f) != (size_t)*len) { fclose(f); free(buf); return NULL; }
    buf[*len] = 0;
    fclose(f);
    return buf;
}

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "用法: es2check <shader> [shader...]\n");
        return 2;
    }

    CGLPixelFormatAttribute attrs[] = {
        kCGLPFAAccelerated, kCGLPFAColorSize, (CGLPixelFormatAttribute)24,
        (CGLPixelFormatAttribute)0,
    };
    CGLPixelFormatObj pix;
    GLint npix;
    if (CGLChoosePixelFormat(attrs, &pix, &npix) != kCGLNoError) {
        fprintf(stderr, "选不到像素格式\n");
        return 2;
    }
    CGLContextObj ctx;
    if (CGLCreateContext(pix, NULL, &ctx) != kCGLNoError) {
        fprintf(stderr, "建不了 GL 上下文\n");
        return 2;
    }
    CGLSetCurrentContext(ctx);

    int bad = 0;
    for (int i = 1; i < argc; i++) {
        long len;
        char *src = slurp(argv[i], &len);
        if (!src) {
            printf("%-28s 读不到\n", argv[i]);
            bad = 1;
            continue;
        }

        // 剥掉 #version 和 precision 行，换成桌面 1.20。
        char *out = malloc(len + 64);
        strcpy(out, "#version 120\n");
        for (char *line = strtok(src, "\n"); line; line = strtok(NULL, "\n")) {
            if (strstr(line, "#version") == line) continue;
            char *t = line;
            while (*t == ' ' || *t == '\t') t++;
            if (strstr(t, "precision ") == t) continue;
            strcat(out, line);
            strcat(out, "\n");
        }

        int frag = strstr(argv[i], ".frag") != NULL;
        GLuint sh = glCreateShader(frag ? GL_FRAGMENT_SHADER : GL_VERTEX_SHADER);
        const char *p = out;
        glShaderSource(sh, 1, &p, NULL);
        glCompileShader(sh);

        GLint ok = 0;
        glGetShaderiv(sh, GL_COMPILE_STATUS, &ok);
        if (ok) {
            printf("%-28s 通过\n", argv[i]);
        } else {
            char log[4096];
            GLsizei n = 0;
            glGetShaderInfoLog(sh, sizeof log, &n, log);
            printf("%-28s 失败\n%s\n", argv[i], log);
            bad = 1;
        }

        glDeleteShader(sh);
        free(src);
        free(out);
    }
    return bad;
}
