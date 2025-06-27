from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from typing import ClassVar as _ClassVar, Mapping as _Mapping, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class Request(_message.Message):
    __slots__ = ("fan", "temp")
    FAN_FIELD_NUMBER: _ClassVar[int]
    TEMP_FIELD_NUMBER: _ClassVar[int]
    fan: FanRequest
    temp: TempRequest
    def __init__(self, fan: _Optional[_Union[FanRequest, _Mapping]] = ..., temp: _Optional[_Union[TempRequest, _Mapping]] = ...) -> None: ...

class FanRequest(_message.Message):
    __slots__ = ("get", "set")
    class Set(_message.Message):
        __slots__ = ("duty",)
        DUTY_FIELD_NUMBER: _ClassVar[int]
        duty: float
        def __init__(self, duty: _Optional[float] = ...) -> None: ...
    class Get(_message.Message):
        __slots__ = ()
        def __init__(self) -> None: ...
    GET_FIELD_NUMBER: _ClassVar[int]
    SET_FIELD_NUMBER: _ClassVar[int]
    get: FanRequest.Get
    set: FanRequest.Set
    def __init__(self, get: _Optional[_Union[FanRequest.Get, _Mapping]] = ..., set: _Optional[_Union[FanRequest.Set, _Mapping]] = ...) -> None: ...

class TempRequest(_message.Message):
    __slots__ = ("get",)
    class Get(_message.Message):
        __slots__ = ()
        def __init__(self) -> None: ...
    GET_FIELD_NUMBER: _ClassVar[int]
    get: TempRequest.Get
    def __init__(self, get: _Optional[_Union[TempRequest.Get, _Mapping]] = ...) -> None: ...

class FanResponse(_message.Message):
    __slots__ = ("duty", "rpm")
    DUTY_FIELD_NUMBER: _ClassVar[int]
    RPM_FIELD_NUMBER: _ClassVar[int]
    duty: float
    rpm: int
    def __init__(self, duty: _Optional[float] = ..., rpm: _Optional[int] = ...) -> None: ...

class TempResponse(_message.Message):
    __slots__ = ("temperature_celsius", "humidity_percent")
    TEMPERATURE_CELSIUS_FIELD_NUMBER: _ClassVar[int]
    HUMIDITY_PERCENT_FIELD_NUMBER: _ClassVar[int]
    temperature_celsius: float
    humidity_percent: float
    def __init__(self, temperature_celsius: _Optional[float] = ..., humidity_percent: _Optional[float] = ...) -> None: ...

class Response(_message.Message):
    __slots__ = ("fan", "temp")
    FAN_FIELD_NUMBER: _ClassVar[int]
    TEMP_FIELD_NUMBER: _ClassVar[int]
    fan: FanResponse
    temp: TempResponse
    def __init__(self, fan: _Optional[_Union[FanResponse, _Mapping]] = ..., temp: _Optional[_Union[TempResponse, _Mapping]] = ...) -> None: ...
