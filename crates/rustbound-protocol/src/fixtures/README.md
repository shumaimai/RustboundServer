# Protocol fixtures

`registry_codec_1_20_1.nbt` is the Join Game registry codec blob for
Minecraft Java Edition 1.20 / 1.20.1 (protocol 763).

It was generated from the publicly published PrismarineJS
`minecraft-data` package (`data/pc/1.20/loginPacket.json` →
`dimensionCodec`), serialized as classic named-root binary NBT
(`TAG_Compound` + empty UTF name + payload), matching 1.20.1
`NbtIo.write` / `FriendlyByteBuf.writeNbt`.

This is protocol observation data, not Mojang proprietary artifacts.
