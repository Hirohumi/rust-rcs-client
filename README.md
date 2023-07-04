# rust-rcs-client
A simple, workable RCS client library.

RCS capabilities are mainly provided by your cellular network. This is a working client side implementation.


### What does it do 主要功能

This library provides most functions required for an RCS application/service. Listed as follows:

这个库提供了一个 RCS 客户端所需要的大部分功能，列举如下：

auto-config

自动配置流程

message send/receive

消息收发

file upload/download

文件上传/下载

Chatbots

聊天机器人


### How to use 如何使用

To use this library in a real device (like a Android mobile), you should compile for the correct architecture. You may want to add a .cargo/config file to tell Rust to use your specific toolchain. It would usually look like this:

在真机上使用时需要注意编译架构。在工作目录添加 .cargo/config 文件可以修改 Rust 在编译到具体架构时所使用的工具链，一般如下：

```
[target.aarch64-linuex-android]
linker = "your clang"

[env]
CC_aarch64-linux-android = "your clang"
AR_aarch64-linux-android = "your llvm-ar"
```

Also, for using .so version of the library under Android, you should specify the soname for the library otherwise the native code will not load, you could do this by changing the build command to the following:

如果要在安卓下面用 .so 版本的库，需要按照以下指令编译，否则会加载不成功：

```
RUSTFLAGS='-C link-arg=-Wl,-soname,librust_rcs_client.so' cargo build --target aarch64-linux-android
```


### What's missing 缺失部分

Although this library will work under most RCS network, I do have to point out there are features missing　that might cause problems under a different network coniguration.

虽然这个库能在大部分 RCS 网络下运行（基本就是中国三大运营商），有一些向前兼容性的功能确实是没有实现的。

For messaging: only cpm standalone mode is supported now.

暂时只支持消息的 standalone 模式。

For file transfer: only http file transfer is supported now.

文件传输也只支持 HTTP 模式（中国移动已经把 MSRP 文件传输下线了）。


### Demo

You can find a demo App here.

https://github.com/Hirohumi/RustyRcs


### Contact

If you have any doubt, contact lydian.ever@gmail.com or QQ:364123445

如有疑问，欢迎联系
