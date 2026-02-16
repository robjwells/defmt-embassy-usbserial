MEMORY
{
  /* 
    These values are confirmed working for S140 7.3.0
    Flash 1MB total:
      - SoftDevice 156K
      - Application 868K

    RAM 256K total:
      - SoftDevice: 128K
      - Application: 128K

    RAM usage for SoftDevice depends on runtime configuration.
  */
  FLASH (rx)     : ORIGIN = 0x00027000, LENGTH = 868K
  RAM (rwx)      : ORIGIN = 0x20020000, LENGTH = 128K
}
